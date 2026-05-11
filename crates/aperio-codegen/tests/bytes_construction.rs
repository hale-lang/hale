//! ws-echo friction `bytes-construction-from-ints` —
//! `std::bytes::from_int(b: Int) -> Bytes` and
//! `std::bytes::concat(a: Bytes, b: Bytes) -> Bytes`. With
//! these plus existing `bytes::at` / `bytes::slice` /
//! `bytes::from_string`, the WebSocket frame writer is
//! straight-line Aperio.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_bytes_construct_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn from_int_yields_single_byte_blob() {
    // 0x81 = 129 — high bit set, would corrupt String surface.
    // bytes::at lets us probe the constructed blob's bytes.
    let src = r#"
        fn main() {
            let b = std::bytes::from_int(0x81);
            let v = std::bytes::at(b, 0);
            println("b0=", v);
        }
    "#;
    let bin = build("from_int", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("b0=129"), "got: {:?}", stdout);
}

#[test]
fn concat_assembles_ws_text_frame_header() {
    // WS unmasked text frame for "hello": 0x81, 0x05, 'h',
    // 'e', 'l', 'l', 'o'. Build via from_int + concat +
    // from_string. Verify each byte position via at().
    let src = r#"
        fn main() {
            let hdr0 = std::bytes::from_int(0x81);     // FIN+text
            let hdr1 = std::bytes::from_int(0x05);     // len=5
            let header = std::bytes::concat(hdr0, hdr1);
            let body = std::bytes::from_string("hello");
            let frame = std::bytes::concat(header, body);
            println("len=", std::bytes::at(frame, 0));
            println("o0=", std::bytes::at(frame, 0));
            println("o1=", std::bytes::at(frame, 1));
            println("o2=", std::bytes::at(frame, 2));
            println("o6=", std::bytes::at(frame, 6));
        }
    "#;
    let bin = build("ws_frame", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("o0=129"), "got: {:?}", stdout);   // 0x81
    assert!(stdout.contains("o1=5"),   "got: {:?}", stdout);   // len
    assert!(stdout.contains("o2=104"), "got: {:?}", stdout);   // 'h'
    assert!(stdout.contains("o6=111"), "got: {:?}", stdout);   // 'o'
}

#[test]
fn concat_empty_with_nonempty_returns_nonempty() {
    // Edge: concat of an empty bytes (via from_string("")) and
    // a non-empty bytes preserves the non-empty content.
    let src = r#"
        fn main() {
            let e = std::bytes::from_string("");
            let nz = std::bytes::from_int(0xAB);
            let r1 = std::bytes::concat(e, nz);
            let r2 = std::bytes::concat(nz, e);
            println("r1.0=", std::bytes::at(r1, 0));
            println("r2.0=", std::bytes::at(r2, 0));
        }
    "#;
    let bin = build("concat_edges", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("r1.0=171"), "got: {:?}", stdout);
    assert!(stdout.contains("r2.0=171"), "got: {:?}", stdout);
}
