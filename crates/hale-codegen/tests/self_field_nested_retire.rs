//! A whole-value `self.<field> = value` store retires the replaced
//! value's nested blocks.
//!
//! The store memcpys the new value's bytes over the field's block, and
//! retired the replaced value's top-level String and Bytes fields
//! (`lotus_str_field_replace_fixup` / `lotus_bytes_field_replace_fixup`).
//! A field pointing at a block of its own — a nested record, a tuple,
//! an out-of-line array — was overwritten and its block left in the
//! locus's arena for the locus's lifetime, with every String and Bytes
//! under it: `self.outer = Outer { n: i, inner: Inner { s: .. } }`
//! grew ~6 MiB over 200 000 replaces (spec/styleguide.md §7, measured on
//! v0.22.0). A tuple or array field also leaked the fresh copy its value
//! was built in before its bytes were copied over the field's block.
//!
//! Now the store walks the replaced value and the new one with the form
//! retire descriptor (`lotus_self_field_retire_replaced`) and retires
//! every block only the old one reached, at the activation boundary —
//! and the field owns every nested block it stores, so no other owner
//! can share a block the walk retires.
//!
//! The flat cases run 100 cycles, dump arena residency, run to 100 000
//! and dump again: the locus arena `H` must hold the same bytes at both
//! points. Before the fix each grew by megabytes. The alias case holds
//! every value read or copied from a replaced field to what it was
//! given, under ASan with chunk recycling off.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const TYPES: &str = r#"
    type Inner { s: String = ""; }
    type Outer { n: Int = 0; inner: Inner = Inner { }; }
    type Mid { m: Int = 0; inner: Inner = Inner { }; b: Bytes = b""; }
    type Deep { n: Int = 0; mid: Mid = Mid { }; tag: String = ""; }
    type Tup { n: Int = 0; t: (Inner, String) = (Inner { }, ""); }
    type Arr { n: Int = 0; a: [Inner; 3] = [Inner { }, Inner { }, Inner { }]; }
    type Bare { n: Int = 0; }
    type Shell { k: Int = 0; bare: Bare = Bare { }; }
"#;

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

/// `fields` are H's params, `put` the body of `put(i: Int)`, which
/// replaces the field with a value built from `i`.
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
    let bin = harness::unique_bin(&format!("hale_self_field_nested_{name}"));
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
fn a_nested_record_retires() {
    // The styleguide's reproducer.
    assert_flat(
        "nested",
        "o: Outer = Outer { };",
        r#"self.o = Outer { n: i, inner: Inner { s: "value-" + to_string(i) } };"#,
    );
}

#[test]
fn a_two_deep_record_retires() {
    // A record in a record in the field, with a Bytes and a String
    // under the middle one and a String at the top.
    assert_flat(
        "two_deep",
        "d: Deep = Deep { };",
        r#"self.d = Deep {
            n: i,
            mid: Mid { m: i, inner: Inner { s: "value-" + to_string(i) }, b: std::bytes::from_string("b-" + to_string(i)) },
            tag: "t" + to_string(i)
        };"#,
    );
}

#[test]
fn a_record_of_scalars_nested_retires() {
    // The nested block owns nothing, but is a block all the same.
    assert_flat("bare", "sh: Shell = Shell { };", "self.sh = Shell { k: i, bare: Bare { n: i } };");
}

#[test]
fn a_tuple_inside_a_record_retires() {
    assert_flat(
        "record_tuple",
        "t: Tup = Tup { };",
        r#"self.t = Tup { n: i, t: (Inner { s: "value-" + to_string(i) }, "x" + to_string(i)) };"#,
    );
}

#[test]
fn an_array_inside_a_record_retires() {
    assert_flat(
        "record_array",
        "a: Arr = Arr { };",
        r#"let v = "value-" + to_string(i);
           self.a = Arr { n: i, a: [Inner { s: v }, Inner { s: v + "b" }, Inner { s: v + "c" }] };"#,
    );
}

#[test]
fn a_tuple_field_retires() {
    // The field itself a tuple: its elements, and the fresh copy the
    // value was built in.
    assert_flat(
        "tuple",
        r#"p: (Inner, Int) = (Inner { }, 0);"#,
        r#"self.p = (Inner { s: "value-" + to_string(i) }, i);"#,
    );
}

#[test]
fn a_tuple_of_scalars_field_retires_its_copy() {
    assert_flat("tuple_scalars", "p: (Int, Int) = (0, 0);", "self.p = (i, i + 1);");
}

#[test]
fn an_array_field_retires() {
    assert_flat(
        "array",
        r#"names: [String; 2] = ["", ""];"#,
        r#"self.names = ["a" + to_string(i), "b" + to_string(i)];"#,
    );
}

#[test]
fn a_nested_field_of_field_store_retires() {
    // The store's slot is a nested record of the locus field.
    assert_flat(
        "field_of_field",
        "d: Deep = Deep { };",
        r#"self.d.mid = Mid { m: i, inner: Inner { s: "value-" + to_string(i) }, b: b"" };"#,
    );
}

#[test]
fn a_read_modify_write_keeps_its_block() {
    // The nested record carried over from the field itself is kept, not
    // copied; the replace around it still holds flat.
    assert_flat(
        "rmw",
        "o: Outer = Outer { inner: Inner { s: \"kept\" } };",
        "self.o = Outer { n: i, inner: self.o.inner };",
    );
}

/// Every value read or copied out of a replaced field keeps what it was
/// given: another field holding a copy of a nested record (whole, as a
/// tuple element, as an array element, through a record that nests
/// it), a field-of-field copy, a vec element, and a local read before
/// the replace in the same activation. After 200 replaces under ASan,
/// with chunk recycling off, every one still reads OLD.
#[test]
fn every_copy_of_a_replaced_nested_block_survives() {
    let src = format!(
        r#"
        {TYPES}
        @form(vec)
        locus Outers {{ capacity {{ heap items of Outer; }} }}
        locus H {{
            params {{
                outer: Outer = Outer {{ }};
                deep: Deep = Deep {{ }};
                keep_inner: Inner = Inner {{ }};
                keep_mid: Mid = Mid {{ }};
                saved: Outer = Outer {{ }};
                again: Deep = Deep {{ }};
                pair: (Inner, Int) = (Inner {{ }}, 0);
                arr: [Inner; 2] = [Inner {{ }}, Inner {{ }}];
                v: Outers = Outers {{ }};
            }}
            fn put(text: String) {{
                self.outer = Outer {{ n: 1, inner: Inner {{ s: text }} }};
                self.deep = Deep {{ n: 1, mid: Mid {{ m: 1, inner: Inner {{ s: text }}, b: std::bytes::from_string(text) }}, tag: text }};
            }}
            fn save() {{
                self.keep_inner = self.outer.inner;
                self.keep_mid = self.deep.mid;
                self.saved = self.outer;
                self.again.mid = self.deep.mid;
                self.pair = (self.outer.inner, 1);
                self.arr = [self.outer.inner, self.deep.mid.inner];
                self.v.push(self.outer);
            }}
            fn swap(text: String) -> String {{
                let inner = self.outer.inner;
                let mid = self.deep.mid;
                self.put(text);
                return inner.s + "/" + mid.inner.s + "/" + std::str::from_bytes(mid.b);
            }}
            fn read() -> String {{
                let first = self.v.get(0) or Outer {{ }};
                return self.keep_inner.s + "/" + self.keep_mid.inner.s + "/" + self.saved.inner.s
                    + "/" + self.again.mid.inner.s + "/" + self.pair.0.s + "/" + self.arr[0].s
                    + "/" + self.arr[1].s + "/" + first.inner.s + "/" + std::str::from_bytes(self.keep_mid.b);
            }}
        }}
        fn main() {{
            let h = H {{ }};
            h.put(std::str::upper("old"));
            h.save();
            println("swap=" + h.swap(std::str::upper("new")));
            let mut i = 0;
            while i < 200 {{ h.put(std::str::upper("new" + to_string(i))); i = i + 1; }}
            println("read=" + h.read());
            println("now=" + h.outer.inner.s + "/" + h.deep.mid.inner.s);
        }}
        "#
    );
    let bin = harness::unique_bin("hale_self_field_nested_alias");
    harness::build_source_asan(&src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    for want in [
        "swap=OLD/OLD/OLD",
        "read=OLD/OLD/OLD/OLD/OLD/OLD/OLD/OLD/OLD",
        "now=NEW199/NEW199",
    ] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}; stderr: {stderr}");
    }
}
