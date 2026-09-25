//! GH #1033: a `String` / `Bytes` self field whose writes change
//! length is held flat.
//!
//! `lotus_str_assign_in_place` / `lotus_bytes_assign_in_place` used to
//! write a SHORTER value into the old block in place. The block's only
//! size record is its length (`strlen`, the Bytes prefix), so the
//! shrink lost it: the next longer write retired the block under the
//! shrunk size — which no request of the real size matches — and
//! allocated afresh. A field alternating between a heap value and an
//! empty one leaked one block per cycle until the locus dissolved (~32
//! bytes per cycle for a 5-byte String). A whole-struct `self.f =
//! Frame { .. }` store had the neighbouring gap: it retired a replaced
//! String field but never a Bytes one.
//!
//! Each case runs 100 cycles, dumps arena residency, runs to 100 000,
//! and dumps again: the locus arena `H` must hold the same bytes at
//! both points. Before the fix every case grew by megabytes.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// The resident bytes of the arena labelled `H`, per residency dump,
/// in order.
fn h_arena_bytes(stderr: &str) -> Vec<u64> {
    stderr
        .lines()
        .filter(|l| l.contains("label=H]"))
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("bytes="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries bytes=N")
        })
        .collect()
}

/// `fields` are H's params, `a` / `b` the two alternating method bodies,
/// `setup` the lets main passes them.
fn assert_flat(name: &str, fields: &str, a: &str, b: &str, args: &str, setup: &str) {
    let src = format!(
        r#"
        type Frame {{ data: Bytes = b""; subject: String = ""; }}
        locus H {{
            params {{ {fields} }}
            fn a({args}) {{ {a} }}
            fn b({args}) {{ {b} }}
        }}
        fn main() {{
            {setup}
            let h = H {{ }};
            let mut i = 0;
            while i < 100 {{ h.a({call}); h.b({call}); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.a({call}); h.b({call}); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
        }}
        "#,
        call = if args.is_empty() { "" } else { "x" },
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_self_field_alt_{name}"));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let dumps = h_arena_bytes(&stderr);
    assert!(dumps.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: H's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

#[test]
fn string_field_heap_then_empty_literal() {
    // The issue's reproducer.
    assert_flat("str_heap_empty", r#"s: String = "";"#,
        r#"self.s = std::str::upper("hello");"#, r#"self.s = "";"#, "", "");
}

#[test]
fn string_field_empty_literal_then_heap() {
    // The reverse order.
    assert_flat("str_empty_heap", r#"s: String = "";"#,
        r#"self.s = "";"#, r#"self.s = std::str::upper("hello");"#, "", "");
}

#[test]
fn string_field_heap_then_zero_length_slice() {
    assert_flat("str_heap_slice0", r#"s: String = "";"#,
        r#"self.s = std::str::upper("hello");"#, "self.s = x[0..0];",
        "x: String", r#"let x = std::str::upper("abc");"#);
}

#[test]
fn string_field_heap_then_upper_of_empty() {
    assert_flat("str_heap_upper_empty", r#"s: String = "";"#,
        r#"self.s = std::str::upper("hello");"#, r#"self.s = std::str::upper("");"#, "", "");
}

#[test]
fn string_field_heap_then_shorter_heap() {
    // Not only an empty value: any shorter write lost the block's size.
    assert_flat("str_heap_short", r#"s: String = "";"#,
        r#"self.s = std::str::upper("hello");"#, r#"self.s = std::str::upper("hi");"#, "", "");
}

#[test]
fn bytes_field_heap_then_empty() {
    assert_flat("bytes_heap_empty", r#"d: Bytes = b"";"#,
        "self.d = std::bytes::slice(x, 0, 5);", r#"self.d = b"";"#,
        "x: Bytes", r#"let x = std::bytes::from_string("abcdefgh");"#);
}

#[test]
fn struct_field_fields_heap_then_empty() {
    assert_flat("frame_fields_heap_empty", "f: Frame = Frame { };",
        r#"self.f.data = std::bytes::slice(x, 0, 5); self.f.subject = std::str::upper("subj.x");"#,
        r#"self.f.data = b""; self.f.subject = "";"#,
        "x: Bytes", r#"let x = std::bytes::from_string("abcdefgh");"#);
}

#[test]
fn whole_struct_replace_retires_its_bytes_field() {
    // The neighbouring gap: `self.f = Frame { .. }` retired a replaced
    // String field but orphaned the Bytes one (8 bytes per write even
    // for an empty payload).
    assert_flat("frame_whole_replace", "f: Frame = Frame { };",
        r#"self.f = Frame { data: b"", subject: "" };"#,
        r#"self.f = Frame { data: std::bytes::slice(x, 0, 5), subject: std::str::upper("subj.x") };"#,
        "x: Bytes", r#"let x = std::bytes::from_string("abcdefgh");"#);
}
