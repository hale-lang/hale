//! C3 (pond follow-up) — `std::crypto::sha256` and
//! `std::crypto::hmac_sha256`. Mirrors the sha1 test pattern;
//! drives pond/crypto off its 140-line pure-Hale O(N²) workaround.
//!
//! Vectors:
//!   - FIPS 180-2 B.1: sha256("abc")
//!   - Empty input: sha256("")
//!   - FIPS 180-2 B.2: sha256(56-byte input)
//!   - RFC 4231 test 1: hmac_sha256(key=0x0b*20, msg="Hi There")
//!
//! The .hl program prints each digest as space-separated decimal
//! byte values; this test crate converts to lowercase hex and
//! compares against the canonical strings.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_crypto_sha256_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// Parse one line of the form `tag= b0 b1 b2 ...` (decimal bytes)
/// into a lowercase hex string. Skips lines that don't start with
/// `tag=`.
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

#[test]
fn sha256_fips_vectors() {
    // Print each digest as "tag= byte byte byte ..." for easy
    // hex reconstruction. 32 bytes per digest.
    let src = r#"
        fn print_digest(name: String, d: Bytes) {
            let mut s = name + "=";
            let mut i = 0;
            while i < 32 {
                let b = std::bytes::at(d, i) or 0;
                s = s + " " + b;
                i = i + 1;
            }
            println(s);
        }

        fn main() {
            // FIPS 180-2 B.1
            let abc = std::bytes::from_string("abc");
            print_digest("abc", std::crypto::sha256(abc));

            // Empty
            let empty = std::bytes::from_string("");
            print_digest("empty", std::crypto::sha256(empty));

            // FIPS 180-2 B.2 — 56-byte input that spans two SHA-256 blocks
            // after padding (forces the multi-block path).
            let b2 = std::bytes::from_string(
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            );
            print_digest("b2", std::crypto::sha256(b2));
        }
    "#;
    let bin = build("fips_vectors", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        extract_hex(&stdout, "abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "empty"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "b2"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        "stdout: {}",
        stdout
    );
}

#[test]
fn hmac_sha256_rfc4231_case_1() {
    // RFC 4231 test 1:
    //   key = 0x0b repeated 20 times
    //   data = "Hi There"
    //   HMAC-SHA-256 = b0344c61d8db38535ca8afceaf0bf12b
    //                  881dc200c9833da726e9376c2e32cff7
    //
    // Build the key by concatenating 20 single-byte Bytes blobs.
    let src = r#"
        fn print_digest(name: String, d: Bytes) {
            let mut s = name + "=";
            let mut i = 0;
            while i < 32 {
                let b = std::bytes::at(d, i) or 0;
                s = s + " " + b;
                i = i + 1;
            }
            println(s);
        }

        fn main() {
            let mut key = std::bytes::from_string("");
            let mut i = 0;
            while i < 20 {
                key = std::bytes::concat(key, std::bytes::from_int(0x0B));
                i = i + 1;
            }
            let msg = std::bytes::from_string("Hi There");
            print_digest("hmac1", std::crypto::hmac_sha256(key, msg));
        }
    "#;
    let bin = build("hmac_rfc1", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        extract_hex(&stdout, "hmac1"),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        "stdout: {}",
        stdout
    );
}
