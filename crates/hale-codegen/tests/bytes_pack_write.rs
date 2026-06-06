//! `BytesBuilder.append_*` binary-pack writers (shm-ring-interop
//! Proposal A, M2). The inverse of `std::bytes::read_*`: append
//! fixed-width scalars (LE/BE, signed/unsigned, f32/f64) plus
//! `append_pad`. Validated by round-trip — write into a builder,
//! snapshot to Bytes, read the fields back with the M1 readers.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_bytes_pack_w_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn round_trip_unsigned_and_floats() {
    let src = r#"
fn main() {
    let b = std::bytes::BytesBuilder { initial_cap: 8 };
    b.append_u8(255);
    b.append_u16_le(513);        // 0x0201
    b.append_u32_be(16909060);   // 0x01020304
    b.append_f64_le(2.5);
    b.append_f32_le(1.5);
    let s = b.snapshot();
    println("len=", to_string(len(s)));
    println("u8=", to_string(std::bytes::read_u8(s, 0) or raise));
    println("u16le=", to_string(std::bytes::read_u16_le(s, 1) or raise));
    println("u32be=", to_string(std::bytes::read_u32_be(s, 3) or raise));
    println("f64=", to_string(std::bytes::read_f64_le(s, 7) or raise));
    println("f32=", to_string(std::bytes::read_f32_le(s, 15) or raise));
}
"#;
    let out = build_and_run("rt", src);
    // 1 + 2 + 4 + 8 + 4 = 19 bytes.
    assert!(out.contains("len=19"), "got {:?}", out);
    assert!(out.contains("u8=255"), "got {:?}", out);
    assert!(out.contains("u16le=513"), "got {:?}", out);
    assert!(out.contains("u32be=16909060"), "got {:?}", out);
    assert!(out.contains("f64=2.5"), "got {:?}", out);
    assert!(out.contains("f32=1.5"), "got {:?}", out);
}

#[test]
fn round_trip_signed_and_pad() {
    let src = r#"
fn main() {
    let b = std::bytes::BytesBuilder { initial_cap: 8 };
    b.append_i8(-1);          // 0xFF
    b.append_pad(4);          // len 1 → +3 zero bytes → len 4
    b.append_i32_le(-2);      // 0xFFFFFFFE
    let s = b.snapshot();
    println("len=", to_string(len(s)));
    println("i8=", to_string(std::bytes::read_i8(s, 0) or raise));
    println("pad1=", to_string(std::bytes::read_u8(s, 1) or raise));
    println("pad3=", to_string(std::bytes::read_u8(s, 3) or raise));
    println("i32le=", to_string(std::bytes::read_i32_le(s, 4) or raise));
}
"#;
    let out = build_and_run("signed_pad", src);
    assert!(out.contains("len=8"), "1 + pad-to-4 + 4 = 8; got {:?}", out);
    assert!(out.contains("i8=-1"), "got {:?}", out);
    assert!(out.contains("pad1=0"), "pad byte; got {:?}", out);
    assert!(out.contains("pad3=0"), "pad byte; got {:?}", out);
    assert!(out.contains("i32le=-2"), "got {:?}", out);
}

#[test]
fn big_endian_byte_order_is_exact() {
    // Append u32 BE then read the individual bytes — confirms order.
    let src = r#"
fn main() {
    let b = std::bytes::BytesBuilder { initial_cap: 8 };
    b.append_u32_be(16909060);   // 0x01020304 → [0x01,0x02,0x03,0x04]
    let s = b.snapshot();
    println("b0=", to_string(std::bytes::read_u8(s, 0) or raise));
    println("b1=", to_string(std::bytes::read_u8(s, 1) or raise));
    println("b2=", to_string(std::bytes::read_u8(s, 2) or raise));
    println("b3=", to_string(std::bytes::read_u8(s, 3) or raise));
}
"#;
    let out = build_and_run("be", src);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"b0=1"), "got {:?}", out);
    assert!(lines.contains(&"b1=2"), "got {:?}", out);
    assert!(lines.contains(&"b2=3"), "got {:?}", out);
    assert!(lines.contains(&"b3=4"), "got {:?}", out);
}
