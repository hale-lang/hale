//! Form cells with enum fields: `@form(hashmap)` and `@form(vec)` cells
//! holding a unit-only enum (an inline `i32` tag) and a payload-bearing
//! enum (a pointer to its `{ i32 tag, body }` storage block).
//!
//! Three faults, from a downstream handoff (friction sweep 2026-10-12):
//!
//! - `@form(hashmap).set` of a cell with ANY enum field was an internal
//!   compiler error. The set's anchor walk treated every enum field as a
//!   pointer, loaded the unit enum's `i32` tag and handed it to the
//!   pointer-shaped deep copy (inkwell's "expected PointerValue" panic).
//!   A unit-only enum now travels with the cell's bytes, like an `Int`.
//! - A payload enum's block was never retired: a replaced or removed
//!   cell left its enum block, and the Strings and Bytes in its payload,
//!   in the form's arena for good (16 bytes a `set` for a `Tag::Empty`).
//!   The retire descriptor now carries a payload enum as a variant
//!   table the walk indexes by the block's tag, and the cell owns its
//!   enum block as it owns a nested record.
//! - `@form(vec).get` of an element with a payload enum field always
//!   failed: the owned copy's variant switch added blocks, and the
//!   result phi named the block the load started in.
//!
//! The churn shapes run 100 cycles, dump arena residency, run to
//! 100 000 and dump again: the form's arena must hold the same bytes.
//! The ASan program reads enum values back after they were shared into
//! another key, held across a replace, stored into a field, read from a
//! serialized map, removed and iterated.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_form_enum_cells_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    stdout
}

/// The friction-log repro: a hashmap cell with a unit-only enum field.
#[test]
fn hashmap_cell_with_a_unit_enum_field_builds_and_reads_back() {
    let out = run(
        "unit",
        r#"
        type Side = enum { Bid, Ask };
        type Order { id: Int = 0; side: Side = Side::Bid; px: Int = 0; }
        @form(hashmap) locus Book { capacity { pool orders of Order indexed_by id; } }
        fn main() {
            let b = Book { };
            b.set(Order { id: 1, side: Side::Ask, px: 10 });
            b.set(Order { id: 2, side: Side::Bid, px: 20 });
            b.set(Order { id: 1, side: Side::Ask, px: 11 });
            let o = b.get(1) or Order { };
            let s = match o.side { Side::Bid -> "bid", Side::Ask -> "ask" };
            b.remove(2) or discard;
            let mut asks = 0;
            for e in b.entries { if e.side == Side::Ask { asks = asks + e.px; } }
            println(s, " ", o.px, " ", b.len(), " ", asks);
        }
        "#,
    );
    assert_eq!(out.trim(), "ask 11 1 11");
}

/// A vec element with a payload enum field reads back through `get`.
#[test]
fn vec_get_of_an_element_with_a_payload_enum_field() {
    let out = run(
        "vec_get",
        r##"
        type Tag = enum { Empty, Named(String), Num(Int) };
        type Order { id: Int = 0; tag: Tag = Tag::Empty; }
        @form(vec) locus V { capacity { heap xs of Order; } }
        fn label(t: Tag) -> String {
            return match t { Tag::Empty -> "-", Tag::Named(s) -> s, Tag::Num(n) -> "#" + to_string(n) };
        }
        fn main() {
            let v = V { };
            v.push(Order { id: 1, tag: Tag::Num(7) });
            v.push(Order { id: 2, tag: Tag::Named(std::str::upper("two")) });
            v.push(Order { id: 3 });
            let a = v.get(0) or Order { id: -1 };
            let b = v.get(1) or Order { id: -1 };
            let c = v.get(2) or Order { id: -1 };
            let d = v.get(3) or Order { id: -1 };
            println(a.id, label(a.tag), " ", b.id, label(b.tag), " ", c.id, label(c.tag), " ", d.id);
        }
        "##,
    );
    assert_eq!(out.trim(), "1#7 2TWO 3- -1");
}

/// The resident bytes of the arena labelled `label`, per residency
/// dump, in order.
fn arena_bytes(stderr: &str, label: &str) -> Vec<u64> {
    let tag = format!("label={label}]");
    stderr
        .lines()
        .filter(|l| l.contains(&tag))
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("used="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries used=N")
        })
        .collect()
}

const TYPES: &str = r#"
    type Side = enum { Bid, Ask };
    type In { s: String = ""; b: Bytes = b""; }
    type Tag = enum { Empty, Named(String), Pair(String, Bytes), Num(Int), Rec(In) };
    type Order { id: Int = 0; side: Side = Side::Bid; tag: Tag = Tag::Empty; }
"#;

/// `decl` declares the form locus `M`; `body` is one cycle of
/// `H.step(x, b)` over `self.m`, with `self.n` counting cycles. The
/// form's arena must hold the same bytes after 100 cycles and after
/// 100 000.
fn assert_form_flat(name: &str, decl: &str, body: &str) {
    let src = format!(
        r#"
        {TYPES}
        {decl}
        locus H {{
            params {{ m: M = M {{ }}; n: Int = 0; }}
            fn step(x: String, b: Bytes) {{ self.n = self.n + 1; {body} }}
        }}
        fn main() {{
            let h = H {{ }};
            let x = std::str::upper("some string of forty characters or so...");
            let b = std::bytes::from_string(x);
            let mut i = 0;
            while i < 100 {{ h.step(x, b); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.step(x, b); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
        }}
        "#
    );
    let bin = harness::unique_bin(&format!("hale_form_enum_cells_flat_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let dumps = arena_bytes(&stderr, "M");
    assert!(dumps.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert!(dumps[0] > 0, "{name}: the form's arena holds nothing; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: the form's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

const MAP: &str = "@form(hashmap) locus M { capacity { pool entries of Order indexed_by id; } }";

#[test]
fn hashmap_replace_of_a_unit_enum_cell_stays_flat() {
    assert_form_flat("unit", MAP, "self.m.set(Order { id: self.n % 4, side: Side::Ask });");
}

#[test]
fn hashmap_replace_frees_a_scalar_payload_enum() {
    assert_form_flat("num", MAP, "self.m.set(Order { id: self.n % 4, tag: Tag::Num(self.n) });");
}

#[test]
fn hashmap_replace_frees_a_string_and_bytes_payload_enum() {
    assert_form_flat("pair", MAP, "self.m.set(Order { id: self.n % 4, tag: Tag::Pair(x, b) });");
}

#[test]
fn hashmap_replace_frees_a_record_payload_enum() {
    assert_form_flat(
        "rec",
        MAP,
        "self.m.set(Order { id: self.n % 4, tag: Tag::Rec(In { s: x, b: b }) });",
    );
}

#[test]
fn hashmap_replace_across_variants_stays_flat() {
    assert_form_flat(
        "mixed",
        MAP,
        "if self.n % 3 == 0 { self.m.set(Order { id: self.n % 4, tag: Tag::Named(x) }); } \
         if self.n % 3 == 1 { self.m.set(Order { id: self.n % 4, tag: Tag::Rec(In { s: x, b: b }) }); } \
         if self.n % 3 == 2 { self.m.set(Order { id: self.n % 4, tag: Tag::Num(self.n) }); }",
    );
}

#[test]
fn hashmap_remove_frees_a_payload_enum() {
    assert_form_flat(
        "remove",
        MAP,
        "self.m.set(Order { id: self.n % 4, tag: Tag::Pair(x, b) }); self.m.remove(self.n % 4) or discard;",
    );
}

#[test]
fn hashmap_get_modify_set_of_a_payload_enum_stays_flat() {
    assert_form_flat(
        "rmw",
        MAP,
        "let o = self.m.get(self.n % 4) or Order { id: self.n % 4, tag: Tag::Named(x) }; \
         self.m.set(Order { id: o.id, side: Side::Ask, tag: o.tag });",
    );
}

#[test]
fn serialized_hashmap_replace_frees_a_payload_enum() {
    assert_form_flat(
        "serialized",
        "@form(hashmap, sync = serialized) locus M { capacity { pool entries of Order indexed_by id; } }",
        "self.m.set(Order { id: self.n % 4, tag: Tag::Pair(x, b) }); \
         let g = self.m.get(self.n % 4) or Order { };",
    );
}

#[test]
fn vec_set_frees_a_payload_enum() {
    assert_form_flat(
        "vec",
        "@form(vec) locus M { capacity { heap xs of Order; } }",
        "if self.m.len() < 4 { self.m.push(Order { id: self.n, tag: Tag::Pair(x, b) }); } \
         else { self.m.set(self.n % 4, Order { id: self.n, tag: Tag::Rec(In { s: x, b: b }) }) or discard; }",
    );
}

/// Enum values read out of cells — shared into another key, held across
/// a replace of their cell, stored into a field, read from a serialized
/// map and from a vec, removed and iterated — stay intact under ASan
/// while the forms free what they replace. The pinned output is what the
/// churn computes, worked out independently by simulating it.
#[test]
fn enum_values_outlive_their_cells() {
    let src = r##"
        type Side = enum { Bid, Ask };
        type Tag = enum { Empty, Named(String), Pair(String, Bytes), Num(Int) };
        type Order { id: Int = 0; side: Side = Side::Bid; tag: Tag = Tag::Empty; }
        @form(hashmap) locus M { capacity { pool orders of Order indexed_by id; } }
        @form(hashmap, sync = serialized) locus SM { capacity { pool orders of Order indexed_by id; } }
        @form(vec) locus V { capacity { heap xs of Order; } }
        fn label(t: Tag) -> String {
            return match t {
                Tag::Empty -> "-",
                Tag::Named(s) -> s,
                Tag::Pair(s, b) -> s + ":" + to_string(len(b)),
                Tag::Num(n) -> "#" + to_string(n),
            };
        }
        fn side(s: Side) -> String {
            return match s { Side::Bid -> "b", Side::Ask -> "a" };
        }
        locus H {
            params { m: M = M { }; sm: SM = SM { }; v: V = V { }; kept: Tag = Tag::Empty; seen: Int = 0; }
            fn make(i: Int, b: Bytes) -> Order {
                let mut t = Tag::Num(i);
                if i % 3 == 0 { t = Tag::Named("n" + to_string(i)); }
                if i % 3 == 1 { t = Tag::Pair("p" + to_string(i), b); }
                let mut s = Side::Ask;
                if i % 2 == 0 { s = Side::Bid; }
                return Order { id: i % 8, side: s, tag: t };
            }
            fn churn(i: Int, b: Bytes) {
                self.m.set(self.make(i, b));
                let a = self.m.get(i % 8) or Order { };
                self.m.set(Order { id: 100 + i % 3, side: a.side, tag: a.tag });
                let g = self.m.get((i + 1) % 8) or Order { };
                self.m.set(self.make(i + 1, b));
                self.seen = self.seen + len(label(g.tag));
                self.kept = g.tag;
                if i % 5 == 0 { self.m.remove(100 + i % 3) or discard; }
                self.sm.set(self.make(i, b));
                let sg = self.sm.get(i % 8) or Order { };
                self.sm.set(self.make(i + 8, b));
                self.seen = self.seen + len(label(sg.tag));
                if self.v.len() < 8 { self.v.push(self.make(i, b)); }
                else {
                    let vg = self.v.get((i + 1) % 8) or Order { };
                    self.v.set(i % 8, self.make(i, b)) or discard;
                    self.v.set((i + 1) % 8, self.make(i + 1, b)) or discard;
                    self.seen = self.seen + len(label(vg.tag));
                }
            }
            fn report() -> String {
                let mut out = "";
                let mut k = 100;
                while k < 103 {
                    let o = self.m.get(k) or Order { tag: Tag::Named("none") };
                    out = out + side(o.side) + label(o.tag) + " ";
                    k = k + 1;
                }
                let z = self.m.get(0) or Order { };
                let s = self.sm.get(3) or Order { };
                let w = self.v.get(5) or Order { };
                let mut n = 0;
                let mut total = 0;
                for e in self.m.entries { n = n + 1; total = total + e.id + len(label(e.tag)); }
                return out + side(z.side) + label(z.tag) + " " + side(s.side) + label(s.tag) + " "
                    + side(w.side) + label(w.tag) + " kept=" + label(self.kept) + " n=" + to_string(n)
                    + " total=" + to_string(total) + " seen=" + to_string(self.seen);
            }
        }
        fn main() {
            let h = H { };
            let b = std::bytes::from_string(std::str::upper("payload-bytes"));
            let mut i = 0;
            while i < 3000 { h.churn(i, b); i = i + 1; }
            println(h.report());
        }
    "##;
    let bin = harness::unique_bin("hale_form_enum_cells_outlive");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert!(
        stdout.contains(
            "an2997 bp2998:13 a#2999 bn3000 an3003 an2997 kept=p2992:13 n=11 total=395 seen=50593"
        ),
        "stdout: {stdout}; stderr: {stderr}"
    );
}
