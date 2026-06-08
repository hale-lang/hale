//! `std::bytes::read_*` binary-pack readers (shm-ring-interop
//! Proposal A). Fixed-width scalar reads at a byte offset, little- and
//! big-endian, signed (sign-extended) and unsigned, plus f32/f64;
//! each `-> Int|Float fallible(IndexError)`, bounds-checked.
//!
//! Test buffers are assembled byte-by-byte with `from_int(b) & 0xFF`
//! + `concat` (the documented "build any byte sequence" idiom), so the
//! exact bytes — including high-bit and IEEE-754 bit patterns — are
//! known.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_bytes_pack_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn integer_readers_le_be_signed_unsigned() {
    // Buffer: [0x01, 0x02, 0x03, 0x04, 0x80, 0xFF] (len 6).
    let src = r#"
fn b6() -> Bytes {
    let a = std::bytes::concat(std::bytes::from_int(1), std::bytes::from_int(2));
    let c = std::bytes::concat(a, std::bytes::from_int(3));
    let d = std::bytes::concat(c, std::bytes::from_int(4));
    let e = std::bytes::concat(d, std::bytes::from_int(128));
    return std::bytes::concat(e, std::bytes::from_int(255));
}
fn main() {
    let b = b6();
    println("u8=", to_string(std::bytes::read_u8(b, 0) or raise));
    println("u16le=", to_string(std::bytes::read_u16_le(b, 0) or raise));
    println("u16be=", to_string(std::bytes::read_u16_be(b, 0) or raise));
    println("u32le=", to_string(std::bytes::read_u32_le(b, 0) or raise));
    println("u32be=", to_string(std::bytes::read_u32_be(b, 0) or raise));
    println("u8_4=", to_string(std::bytes::read_u8(b, 4) or raise));
    println("i8_4=", to_string(std::bytes::read_i8(b, 4) or raise));
    println("i8_5=", to_string(std::bytes::read_i8(b, 5) or raise));
    println("i16le_4=", to_string(std::bytes::read_i16_le(b, 4) or raise));
}
"#;
    let out = build_and_run("ints", src);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"u8=1"), "got {:?}", out);
    assert!(lines.contains(&"u16le=513"), "0x0201; got {:?}", out);
    assert!(lines.contains(&"u16be=258"), "0x0102; got {:?}", out);
    assert!(lines.contains(&"u32le=67305985"), "0x04030201; got {:?}", out);
    assert!(lines.contains(&"u32be=16909060"), "0x01020304; got {:?}", out);
    assert!(lines.contains(&"u8_4=128"), "got {:?}", out);
    assert!(lines.contains(&"i8_4=-128"), "0x80 sign-extended; got {:?}", out);
    assert!(lines.contains(&"i8_5=-1"), "0xFF sign-extended; got {:?}", out);
    // [0x80, 0xFF] LE = 0xFF80 = -128 as i16.
    assert!(lines.contains(&"i16le_4=-128"), "got {:?}", out);
}

#[test]
fn float_readers() {
    // f64 1.0 = 0x3FF0000000000000 → LE bytes [0,0,0,0,0,0,0xF0,0x3F].
    // f32 2.5 = 0x40200000        → LE bytes [0,0,0x20,0x40].
    let src = r#"
fn append(b: Bytes, n: Int) -> Bytes {
    return std::bytes::concat(b, std::bytes::from_int(n));
}
fn main() {
    // f64 1.0
    let mut d = std::bytes::from_int(0);
    d = append(d, 0); d = append(d, 0); d = append(d, 0);
    d = append(d, 0); d = append(d, 0); d = append(d, 240); d = append(d, 63);
    println("f64=", to_string(std::bytes::read_f64_le(d, 0) or raise));

    // f32 2.5
    let mut f = std::bytes::from_int(0);
    f = append(f, 0); f = append(f, 32); f = append(f, 64);
    println("f32=", to_string(std::bytes::read_f32_le(f, 0) or raise));
}
"#;
    let out = build_and_run("floats", src);
    assert!(out.contains("f64=1"), "expected 1.0; got {:?}", out);
    assert!(out.contains("f32=2.5"), "expected 2.5; got {:?}", out);
}

#[test]
fn out_of_bounds_read_raises_index_error() {
    // Buffer of 6 bytes; read_u32_le at offset 4 needs [4,8) > 6 → OOB.
    let src = r#"
fn diag(e: IndexError) -> Int {
    println("kind=", e.kind, " index=", to_string(e.index), " len=", to_string(e.len));
    return -1;
}
fn b6() -> Bytes {
    let a = std::bytes::concat(std::bytes::from_int(1), std::bytes::from_int(2));
    let c = std::bytes::concat(a, std::bytes::from_int(3));
    let d = std::bytes::concat(c, std::bytes::from_int(4));
    let e = std::bytes::concat(d, std::bytes::from_int(5));
    return std::bytes::concat(e, std::bytes::from_int(6));
}
fn main() {
    let b = b6();
    let x = std::bytes::read_u32_le(b, 4) or diag(err);
    println("x=", to_string(x));
    // In-bounds read at the same width still works.
    println("ok=", to_string(std::bytes::read_u16_le(b, 4) or raise));
}
"#;
    let out = build_and_run("oob", src);
    assert!(
        out.contains("kind=out_of_bounds") && out.contains("index=4") && out.contains("len=6"),
        "expected IndexError fields on OOB; got {:?}",
        out
    );
    assert!(out.contains("x=-1"), "fallback should fire; got {:?}", out);
    // [0x05, 0x06] LE = 0x0605 = 1541.
    assert!(out.contains("ok=1541"), "in-bounds read; got {:?}", out);
}

#[test]
fn oob_offsets_take_indexerror_path_not_oob_read() {
    // Hardening regression (2026-06-08): every read whose [off, off+width)
    // exceeds the buffer — including off == i64::MAX, which made the old
    // `off + width > len` guard overflow (signed UB; on wrap it went
    // negative and *passed* the guard → OOB read) — must take the
    // IndexError path and substitute the sentinel, never read OOB.
    // Run under UBSan (LOTUS_UBSAN=1) to catch the overflow directly.
    let src = r#"
fn b6() -> Bytes {
    let a = std::bytes::concat(std::bytes::from_int(1), std::bytes::from_int(2));
    let c = std::bytes::concat(a, std::bytes::from_int(3));
    let d = std::bytes::concat(c, std::bytes::from_int(4));
    let e = std::bytes::concat(d, std::bytes::from_int(128));
    return std::bytes::concat(e, std::bytes::from_int(255));
}
fn main() {
    let b = b6();
    println("max=", to_string(std::bytes::read_u32_le(b, 9223372036854775807) or -1));
    println("atlen=", to_string(std::bytes::read_u8(b, 6) or -1));
    println("straddle=", to_string(std::bytes::read_u32_le(b, 4) or -1));
    println("boundok=", to_string(std::bytes::read_u16_le(b, 4) or -1));
}
"#;
    let out = build_and_run("oob", src);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"max=-1"),
        "off==i64::MAX must hit IndexError (the overflow case), not OOB-read; got {:?}", out);
    assert!(lines.contains(&"atlen=-1"),
        "off==len must hit IndexError; got {:?}", out);
    assert!(lines.contains(&"straddle=-1"),
        "off+width past end must hit IndexError; got {:?}", out);
    // Sanity: a read that fits at the boundary (off 4, width 2, len 6)
    // still succeeds — the guard rejects only genuine OOB.
    assert!(lines.contains(&"boundok=65408"),
        "valid boundary read [0x80,0xFF] LE must succeed; got {:?}", out);
}
