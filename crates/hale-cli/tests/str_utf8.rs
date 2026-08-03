//! UTF-8 code-point decoding (#353 item 7).
//!
//! ## Scope, stated because unicode is where "add a feature" quietly
//! ## becomes five commitments
//!
//! This provides code-point decoding and nothing else: `cp_count`,
//! `cp_at` (by byte offset) and `cp_size`. Hale's `String` stays
//! BYTE-oriented; these let a caller walk code points deliberately
//! rather than pretending bytes are characters.
//!
//! Normalization (NFC/NFD), case folding beyond ASCII, grapheme
//! cluster segmentation and locale-aware collation are each a
//! separate commitment carrying its own tables, and those tables are
//! megabytes against a wasm target that ships whatever it is given.
//! Half-shipping them is worse than not shipping them — a `to_upper`
//! that works for ASCII and silently mangles Turkish dotted I is a
//! correctness bug that reads like a feature.
//!
//! Invalid UTF-8 yields -1 rather than U+FFFD, so a caller cannot
//! mistake corruption for content.

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir()
        .join(format!("hale-u8-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("run")
        .arg(&f)
        .output()
        .expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Code points, not bytes. `"héllo"` is 5 code points in 6 bytes, and
/// conflating the two is the bug this exists to let people avoid.
#[test]
fn counts_code_points_not_bytes() {
    let out = run(
        "fn main() {\n\
             println(std::str::cp_count(\"héllo\"));\n\
             println(std::str::cp_count(\"日本語\"));\n\
             println(std::str::cp_count(\"\"));\n\
         }",
        "count",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["5", "3", "0"],
        "héllo is 5 code points in 6 bytes; 日本語 is 3 in 9: {:?}",
        got
    );
}

#[test]
fn decodes_each_width() {
    let out = run(
        "fn main() {\n\
             println(std::str::cp_at(\"héllo\", 0));\n\
             println(std::str::cp_at(\"héllo\", 1));\n\
             println(std::str::cp_at(\"日本語\", 0));\n\
             println(std::str::cp_size(\"héllo\", 0));\n\
             println(std::str::cp_size(\"héllo\", 1));\n\
             println(std::str::cp_size(\"日本語\", 0));\n\
         }",
        "decode",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        // 'h' = 104; 'é' = U+00E9 = 233; '日' = U+65E5 = 26085
        vec!["104", "233", "26085", "1", "2", "3"],
        "1-, 2- and 3-byte sequences must all decode: {:?}",
        got
    );
}

/// A byte offset landing mid-sequence is an error, not a plausible
/// value. Returning something decodable from a continuation byte
/// would let a caller walk a string wrongly and never find out.
#[test]
fn a_continuation_byte_is_rejected() {
    let out = run(
        "fn main() {\n\
             println(std::str::cp_size(\"é\", 1));\n\
             println(std::str::cp_at(\"é\", 1));\n\
         }",
        "midseq",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["-1", "-1"],
        "offset 1 is inside a 2-byte sequence: {:?}",
        got
    );
}

/// Pure — usable from a `@deterministic @no_syscall` fn.
#[test]
fn decoding_is_pure() {
    let out = run(
        "@deterministic @no_syscall\n\
         fn n(s: String) -> Int { return std::str::cp_count(s); }\n\
         fn main() { println(n(\"日本語\")); }",
        "pure",
    );
    assert_eq!(out.trim(), "3", "must certify as pure: {}", out);
}
