//! 2026-05-27 — `std::crypto::crc32`. IEEE 802.3 reversed
//! polynomial (`0xEDB88320`), init `0xFFFFFFFF`, final XOR
//! `0xFFFFFFFF` — the variant zlib's `crc32()` and Python's
//! `binascii.crc32` return.
//!
//! Vectors from RFC 1952 / zlib reference (all standard):
//!     crc32("")          = 0x00000000
//!     crc32("a")         = 0xE8B7BE43
//!     crc32("abc")       = 0x352441C2
//!     crc32("123456789") = 0xCBF43926

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_crypto_crc32_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn crc32_standard_vectors() {
    let src = r#"
        fn main() {
            let empty = std::bytes::from_string("");
            println("empty=", std::crypto::crc32(empty));

            let a = std::bytes::from_string("a");
            println("a=", std::crypto::crc32(a));

            let abc = std::bytes::from_string("abc");
            println("abc=", std::crypto::crc32(abc));

            let nines = std::bytes::from_string("123456789");
            println("123456789=", std::crypto::crc32(nines));
        }
    "#;
    let (out, status) = build_and_run("vectors", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);

    // Decimal forms of the canonical CRC32 values.
    assert!(out.contains("empty=0"),         "got: {:?}", out);
    assert!(out.contains("a=3904355907"),    "got: {:?}", out);   // 0xE8B7BE43
    assert!(out.contains("abc=891568578"),   "got: {:?}", out);   // 0x352441C2
    assert!(out.contains("123456789=3421780262"), "got: {:?}", out); // 0xCBF43926
}
