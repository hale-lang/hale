//! A whole-value store into a payload-enum locus field retires what the
//! replaced value owned.
//!
//! The field points at the enum's storage block, and `self.e =
//! E::A(s)` deep-copied the value into a fresh block of the locus's
//! arena, then copied its bytes over the field's block: the fresh copy
//! and the replaced value's payload (a String, a Bytes, a nested record)
//! stayed in the arena for the locus's lifetime, ~32 bytes a replace
//! (3 MiB over 100 000). A nested payload enum already retired with the
//! record holding it (#1542); the field itself did not.
//!
//! Now the store walks the old value with its own variant's descriptor
//! and the new one with its own (`lotus_self_field_retire_replaced_enum`)
//! and retires the payload only the old one reached, and the fresh
//! copy, at the activation boundary. The flat cases dump arena residency
//! after 100 and 100 000 replaces; the alias case reads every copy of a
//! replaced value under ASan with chunk recycling off.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const TYPES: &str = r#"
    type Inner { s: String = ""; }
    type E = enum { A(String), B(Int), C };
    type P = enum { Rec(Inner, Int), Two(String, Bytes), Nil };
    type F { n: Int = 0; tag: E = E::C; }
"#;

/// The resident bytes of the arena labelled `H`, per residency dump.
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

/// `fields` are H's params, `put` the body of `put(i: Int)`.
fn assert_flat(name: &str, fields: &str, put: &str) {
    let src = format!(
        r#"
        {TYPES}
        locus H {{
            params {{ {fields} }}
            fn put(i: Int) {{ {put} }}
        }}
        fn main() {{
            let h = H {{ }};
            let mut i = 0;
            while i < 100 {{ h.put(i); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.put(i); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
        }}
        "#
    );
    let bin = harness::unique_bin(&format!("hale_self_field_enum_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
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
        "{name}: H's arena grew between 100 and 100000 replaces; stderr: {stderr}"
    );
}

#[test]
fn a_string_payload_enum_field_retires() {
    assert_flat("string", "e: E = E::C;", r#"self.e = E::A("value-" + to_string(i));"#);
}

#[test]
fn a_fixed_length_payload_recycles_its_blocks() {
    // The payload String and the fresh enum block are both small blocks;
    // first fit handed the String the enum's block and stranded both.
    assert_flat("fixed", "e: E = E::C;", r#"self.e = E::A("value-" + to_string(i % 10));"#);
}

#[test]
fn a_field_switching_variants_retires() {
    assert_flat(
        "switch",
        "e: E = E::C;",
        r#"if i % 3 == 0 { self.e = E::A("value-" + to_string(i)); }
           else if i % 3 == 1 { self.e = E::B(i); }
           else { self.e = E::C; }"#,
    );
}

#[test]
fn a_record_and_bytes_payload_retires() {
    assert_flat(
        "record_bytes",
        "p: P = P::Nil;",
        r#"if i % 2 == 0 { self.p = P::Rec(Inner { s: "value-" + to_string(i) }, i); }
           else { self.p = P::Two("t" + to_string(i), std::bytes::from_string("b" + to_string(i))); }"#,
    );
}

#[test]
fn a_nested_enum_field_of_field_store_retires() {
    assert_flat("field_of_field", "f: F = F { };", r#"self.f.tag = E::A("value-" + to_string(i));"#);
}

/// Every copy of a replaced enum value keeps what it was given, under
/// ASan with chunk recycling off: another field holding it whole, a
/// record field holding it, a String field holding its payload, a `let`
/// of it held across the replace, a payload matched out of the field
/// and held across a callee that replaces it, and a payload built from
/// another field (`E::A(self.name)`), which the replace must not free
/// under that field.
#[test]
fn every_copy_of_a_replaced_enum_survives() {
    let src = format!(
        r#"
        {TYPES}
        type Box {{ e: E = E::C; }}
        locus H {{
            params {{
                e: E = E::C;
                saved: E = E::C;
                boxed: Box = Box {{ }};
                text: String = "";
                name: String = "";
            }}
            fn put(t: String) {{ self.e = E::A(t); }}
            fn churn() {{
                let mut i = 0;
                while i < 200 {{ self.put(std::str::upper("new" + to_string(i))); i = i + 1; }}
            }}
            fn save() {{
                self.saved = self.e;
                self.boxed = Box {{ e: self.e }};
                self.text = match self.e {{ E::A(s) -> s, E::B(_) -> "b", E::C -> "c", }};
            }}
            fn held() -> String {{
                let x = self.e;
                self.churn();
                return match x {{ E::A(s) -> s, E::B(_) -> "b", E::C -> "c", }};
            }}
            fn matched() -> String {{
                return match self.e {{
                    E::A(s) -> {{ self.churn(); s }},
                    E::B(_) -> "b",
                    E::C -> "c",
                }};
            }}
            fn from_field() {{
                self.name = std::str::upper("named");
                self.e = E::A(self.name);
                self.churn();
            }}
            fn read() -> String {{
                let a = match self.saved {{ E::A(s) -> s, E::B(_) -> "b", E::C -> "c", }};
                let b = match self.boxed.e {{ E::A(s) -> s, E::B(_) -> "b", E::C -> "c", }};
                return a + "/" + b + "/" + self.text;
            }}
        }}
        fn main() {{
            let h = H {{ }};
            h.put(std::str::upper("old"));
            h.save();
            h.churn();
            println("read=" + h.read());
            h.put(std::str::upper("old"));
            println("held=" + h.held());
            h.put(std::str::upper("old"));
            println("matched=" + h.matched());
            h.from_field();
            println("name=" + h.name);
        }}
        "#
    );
    let bin = harness::unique_bin("hale_self_field_enum_alias");
    harness::build_source_asan(&src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    for want in ["read=OLD/OLD/OLD", "held=OLD", "matched=OLD", "name=NAMED"] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}; stderr: {stderr}");
    }
}
