//! #4 of the fast-protocol-I/O substrate plan: `std::bytes::find_byte`
//! (word-at-a-time scan, the length/delimiter-framing primitive) and
//! `BytesBuilder.xor_mask` / `std::bytes::builder::__xor_mask_into` (block
//! XOR masking — the WebSocket primitive that replaces a per-byte `from_int`
//! + append loop).

use hale_codegen::build_executable;
use std::process::Command;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_bytes_xf_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn find_byte_locates_and_reports_absence() {
    // "hello,world": ',' (44) at index 5; '!' (33) absent; 'o' (111) at
    // indices 4 and 7, so a scan from off=6 finds index 7.
    let src = r#"
        fn main() {
            let b = std::bytes::from_string("hello,world");
            println("comma=", std::bytes::find_byte(b, 0, 44));
            println("none=", std::bytes::find_byte(b, 0, 33));
            println("from6=", std::bytes::find_byte(b, 6, 111));
        }
    "#;
    let (out, status) = build_and_run("find", src);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    assert!(out.contains("comma=5"), "got: {:?}", out);
    assert!(out.contains("none=-1"), "got: {:?}", out);
    assert!(out.contains("from6=7"), "got: {:?}", out);
}

#[test]
fn xor_mask_masks_and_round_trips() {
    // key = 0x12345678; low byte 0x78. payload[0]='t' (0x74) →
    // masked[0] = 0x74 ^ 0x78 = 0x0C = 12. XOR is involutive, so masking
    // the masked bytes with the same key restores the original.
    let src = r#"
        fn main() {
            let payload = std::bytes::from_string("the-quick-brown-fox");
            let key = 305419896;
            let mb = std::bytes::BytesBuilder { };
            mb.xor_mask(payload, key);
            let m = mb.finish();
            println("mask0=", std::bytes::at(m, 0) or raise);
            let ub = std::bytes::BytesBuilder { };
            ub.xor_mask(m, key);
            let u = ub.finish();
            println("roundtrip=", std::str::from_bytes(u));
        }
    "#;
    let (out, status) = build_and_run("xor", src);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    assert!(out.contains("mask0=12"), "block-XOR must mask byte 0 to 0x74^0x78=12; got: {:?}", out);
    assert!(
        out.contains("roundtrip=the-quick-brown-fox"),
        "mask∘mask with the same key must restore the original; got: {:?}",
        out
    );
}
