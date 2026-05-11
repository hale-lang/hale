//! ws-echo friction `sha1-base64-missing` — `std::crypto::sha1`
//! and `std::text::base64::encode`. RFC 6455 §1.3 specifies the
//! WebSocket Sec-WebSocket-Accept handshake derivation:
//!     base64(sha1(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))
//! With the known key `dGhlIHNhbXBsZSBub25jZQ==`, the expected
//! Accept value is `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_sha1_base64_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn sha1_known_test_vectors() {
    // FIPS 180-1 SHA-1 test vectors:
    //   "abc" → a9993e36 4706816a ba3e2571 7850c26c 9cd0d89d
    let src = r#"
        fn main() {
            let abc = std::bytes::from_string("abc");
            let d = std::crypto::sha1(abc);
            println("b0=", std::bytes::at(d, 0));
            println("b1=", std::bytes::at(d, 1));
            println("b2=", std::bytes::at(d, 2));
            println("b3=", std::bytes::at(d, 3));
            println("b19=", std::bytes::at(d, 19));
        }
    "#;
    let bin = build("abc", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // a9 99 3e 36 ... 9d (last byte)
    assert!(stdout.contains("b0=169"), "got: {:?}", stdout);  // 0xa9
    assert!(stdout.contains("b1=153"), "got: {:?}", stdout);  // 0x99
    assert!(stdout.contains("b2=62"),  "got: {:?}", stdout);  // 0x3e
    assert!(stdout.contains("b3=54"),  "got: {:?}", stdout);  // 0x36
    assert!(stdout.contains("b19=157"),"got: {:?}", stdout);  // 0x9d
}

#[test]
fn base64_known_test_vectors() {
    // RFC 4648 §10:
    //   ""       → ""
    //   "f"      → "Zg=="
    //   "fo"     → "Zm8="
    //   "foo"    → "Zm9v"
    //   "foobar" → "Zm9vYmFy"
    let src = r#"
        fn main() {
            let a = std::bytes::from_string("");
            let b = std::bytes::from_string("f");
            let c = std::bytes::from_string("fo");
            let d = std::bytes::from_string("foo");
            let e = std::bytes::from_string("foobar");
            println("a=", std::text::base64::encode(a));
            println("b=", std::text::base64::encode(b));
            println("c=", std::text::base64::encode(c));
            println("d=", std::text::base64::encode(d));
            println("e=", std::text::base64::encode(e));
        }
    "#;
    let bin = build("rfc_vectors", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a="), "empty: {:?}", stdout);
    assert!(stdout.contains("b=Zg=="), "got: {:?}", stdout);
    assert!(stdout.contains("c=Zm8="), "got: {:?}", stdout);
    assert!(stdout.contains("d=Zm9v"), "got: {:?}", stdout);
    assert!(stdout.contains("e=Zm9vYmFy"), "got: {:?}", stdout);
}

#[test]
fn ws_handshake_derivation_round_trips() {
    // RFC 6455 §1.3: with Sec-WebSocket-Key
    // `dGhlIHNhbXBsZSBub25jZQ==`, the Accept value is
    // `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`.
    let src = r#"
        fn main() {
            let key = std::bytes::from_string("dGhlIHNhbXBsZSBub25jZQ==");
            let guid = std::bytes::from_string(
                "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
            );
            let combined = std::bytes::concat(key, guid);
            let digest = std::crypto::sha1(combined);
            let acc = std::text::base64::encode(digest);
            println("accept=", acc);
        }
    "#;
    let bin = build("ws_accept", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("accept=s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
        "got: {:?}",
        stdout
    );
}
