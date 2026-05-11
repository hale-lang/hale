//! ws-echo friction `bitwise-int-binops-not-lowered` —
//! wire `&`, `|`, `^`, `<<`, `>>` on Int through to LLVM's
//! native ops. Parser already accepts; typechecker already
//! types as Int; only codegen was missing.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_int_bitwise_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn all_five_bitwise_ops_round_trip() {
    let src = r#"
        fn main() {
            let a = 0xF0;     // 240
            let b = 0x0F;     //  15
            println("and=", a & b);    //   0
            println("or=",  a | b);    // 255
            println("xor=", a ^ b);    // 255
            let c = 1;
            println("shl=", c << 4);   //  16
            let d = 256;
            println("shr=", d >> 4);   //  16
        }
    "#;
    let bin = build("all_five", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("and=0"),   "got: {:?}", stdout);
    assert!(stdout.contains("or=255"),  "got: {:?}", stdout);
    assert!(stdout.contains("xor=255"), "got: {:?}", stdout);
    assert!(stdout.contains("shl=16"),  "got: {:?}", stdout);
    assert!(stdout.contains("shr=16"),  "got: {:?}", stdout);
}

#[test]
fn ws_frame_header_bit_extraction() {
    // The actual ws-echo motivating pattern: pull FIN bit,
    // opcode nibble, MASK bit, len7 from a single byte.
    let src = r#"
        fn main() {
            let b0 = 0x81;          // FIN + text-frame opcode
            let b1 = 0x85;          // MASK + len7=5
            let fin = (b0 & 0x80) != 0;
            let opcode = b0 & 0x0F;
            let mask = (b1 & 0x80) != 0;
            let len7 = b1 & 0x7F;
            if fin { println("fin=true"); }
            println("opcode=", opcode);
            if mask { println("mask=true"); }
            println("len7=", len7);
        }
    "#;
    let bin = build("ws_header", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fin=true"),  "got: {:?}", stdout);
    assert!(stdout.contains("opcode=1"),  "got: {:?}", stdout);
    assert!(stdout.contains("mask=true"), "got: {:?}", stdout);
    assert!(stdout.contains("len7=5"),    "got: {:?}", stdout);
}
