//! C10 (pond follow-up): binary-safe builder family
//! `std::bytes::builder_new` / `builder_append` / `builder_len` /
//! `builder_finish`. Mirror of the str-builder family but with
//! Bytes ABI on both sides — embedded NUL bytes survive end-to-end.
//!
//! Coverage:
//!  - 0-byte: new + immediate finish returns empty Bytes.
//!  - Round-trip of chunks containing embedded NULs and high
//!    bytes (0x80..0xff) — the truncate-on-NUL hazard the
//!    str-builder family would hit.
//!  - `builder_len` tracks the running accumulator.
//!
//! Test chunks are assembled via `std::bytes::from_int(byte)` +
//! `std::bytes::concat(...)` because:
//!   1. `std::bytes::from_string(...)` strlens its input, which
//!      truncates at the first embedded NUL — exactly the hazard
//!      this surface exists to fix.
//!   2. The lexer rejects `\xNN` for NN > 0x7f inside string
//!      literals (high-byte escape route would UTF-8-encode and
//!      surprise the caller), and this worktree predates `b"..."`.
//! So per-byte construction via from_int is the canonical route
//! for these tests.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_bytes_builder_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn empty_builder_finishes_to_zero_length_bytes() {
    // new + immediate finish: the accumulator never sees an
    // append; the resulting blob must report len=0 so downstream
    // code can shape its empty-case branch off the explicit length.
    let src = r#"
        fn main() {
            let b = std::bytes::builder_new();
            let out = std::bytes::builder_finish(b);
            println("len=", len(out));
        }
    "#;
    let bin = build("empty", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn round_trip_preserves_embedded_nul_bytes() {
    // The whole point of this surface: chunks with embedded NULs
    // (and high bytes 0x80..0xff that the String path would
    // either corrupt or refuse to lex) must survive append +
    // finish. Two chunks of three bytes each, totalling six:
    //   chunk 1: 0x00, 0x01, 0x02  (NUL up front)
    //   chunk 2: 0xff, 0xfe, 0x00  (high bytes + trailing NUL)
    // Per-byte assembly via from_int + concat — see file-header
    // note for why this is the only route in this worktree.
    let src = r#"
        fn build_chunk(b0: Int, b1: Int, b2: Int) -> Bytes {
            let p0 = std::bytes::from_int(b0);
            let p1 = std::bytes::from_int(b1);
            let p2 = std::bytes::from_int(b2);
            return std::bytes::concat(std::bytes::concat(p0, p1), p2);
        }

        fn main() {
            let c1 = build_chunk(0x00, 0x01, 0x02);
            let c2 = build_chunk(0xff, 0xfe, 0x00);

            let b = std::bytes::builder_new();
            let _ = std::bytes::builder_append(b, c1);
            let _ = std::bytes::builder_append(b, c2);
            let out = std::bytes::builder_finish(b);

            println("len=", len(out));
            println("b0=", std::bytes::at(out, 0) or -1);
            println("b1=", std::bytes::at(out, 1) or -1);
            println("b2=", std::bytes::at(out, 2) or -1);
            println("b3=", std::bytes::at(out, 3) or -1);
            println("b4=", std::bytes::at(out, 4) or -1);
            println("b5=", std::bytes::at(out, 5) or -1);
        }
    "#;
    let bin = build("nuls", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=6"), "len wrong: {:?}", stdout);
    // Byte-by-byte: NULs at positions 0 and 5; 0xff and 0xfe
    // surface as 255 / 254 (unsigned int read).
    assert!(stdout.contains("b0=0"),   "b0: {:?}", stdout);
    assert!(stdout.contains("b1=1"),   "b1: {:?}", stdout);
    assert!(stdout.contains("b2=2"),   "b2: {:?}", stdout);
    assert!(stdout.contains("b3=255"), "b3: {:?}", stdout);
    assert!(stdout.contains("b4=254"), "b4: {:?}", stdout);
    assert!(stdout.contains("b5=0"),   "b5: {:?}", stdout);
}

#[test]
fn builder_len_tracks_running_count() {
    // After each append, builder_len() must reflect the cumulative
    // byte count — chunks containing NULs and high bytes count
    // the same as any other byte (the whole point of moving off
    // strlen-based str-builder accumulation).
    //
    // Three appends:
    //   chunk A: 3 ASCII bytes  ('a' = 0x61, 'b' = 0x62, 'c' = 0x63)
    //   chunk B: 2 NUL bytes    (0x00, 0x00)
    //   chunk C: 4 high bytes   (0xff, 0xff, 0xff, 0xff)
    // Running counts: 0, 3, 5, 9.
    let src = r#"
        fn build_chunk3(b0: Int, b1: Int, b2: Int) -> Bytes {
            let p0 = std::bytes::from_int(b0);
            let p1 = std::bytes::from_int(b1);
            let p2 = std::bytes::from_int(b2);
            return std::bytes::concat(std::bytes::concat(p0, p1), p2);
        }
        fn build_chunk2(b0: Int, b1: Int) -> Bytes {
            let p0 = std::bytes::from_int(b0);
            let p1 = std::bytes::from_int(b1);
            return std::bytes::concat(p0, p1);
        }
        fn build_chunk4(b0: Int, b1: Int, b2: Int, b3: Int) -> Bytes {
            let p0 = std::bytes::from_int(b0);
            let p1 = std::bytes::from_int(b1);
            let p2 = std::bytes::from_int(b2);
            let p3 = std::bytes::from_int(b3);
            let lo = std::bytes::concat(p0, p1);
            let hi = std::bytes::concat(p2, p3);
            return std::bytes::concat(lo, hi);
        }

        fn main() {
            let ca = build_chunk3(0x61, 0x62, 0x63);
            let cb = build_chunk2(0x00, 0x00);
            let cc = build_chunk4(0xff, 0xff, 0xff, 0xff);

            let b = std::bytes::builder_new();
            println("l0=", std::bytes::builder_len(b));
            let _ = std::bytes::builder_append(b, ca);
            println("l1=", std::bytes::builder_len(b));
            let _ = std::bytes::builder_append(b, cb);
            println("l2=", std::bytes::builder_len(b));
            let _ = std::bytes::builder_append(b, cc);
            println("l3=", std::bytes::builder_len(b));
            let out = std::bytes::builder_finish(b);
            println("final=", len(out));
        }
    "#;
    let bin = build("len_track", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("l0=0"), "l0: {:?}", stdout);
    assert!(stdout.contains("l1=3"), "l1: {:?}", stdout);
    assert!(stdout.contains("l2=5"), "l2: {:?}", stdout);
    assert!(stdout.contains("l3=9"), "l3: {:?}", stdout);
    assert!(stdout.contains("final=9"), "final: {:?}", stdout);
}
