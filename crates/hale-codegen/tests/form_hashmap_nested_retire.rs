//! `@form(hashmap)` reclaims everything a replaced or removed cell owns.
//!
//! The map's retire (`lotus_hashmap_retire_cell`) walked the cell's
//! top-level String fields only. A replaced or removed cell's Bytes, and
//! its nested records, tuples and out-of-line arrays (each block and its
//! own String and Bytes), stayed in the map's arena for good: a cell
//! holding a nested record carrying a String and Bytes grew the arena
//! 65,536 -> 11,206,656 bytes over 100,000 sets. The map now installs
//! the form retire descriptor `@form(vec)` uses (GH #1518), so the
//! deferred retire frees the whole value, at any depth.
//!
//! Freeing a nested block needs the cell to own it. The set's anchor
//! walk used to pass a nested block through when it already lived in the
//! map's arena, so a value read from one cell and stored under another
//! key left two cells sharing it; the walk now copies it unless it is
//! the block the same key's cell already holds at the same field (the
//! get-modify-set shape, which stays allocation-free). A synced map's
//! reads copy the nested blocks and Bytes out, as they did its Strings.
//!
//! Each arena shape runs 100 cycles, dumps arena residency, runs to
//! 100 000, and dumps again: the map's own arena (labelled with its locus
//! name) must hold the same bytes at both points. The ASan programs check
//! the other side — what was read out of a cell, shared from it, carried
//! over or held across a replace stays intact.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The resident bytes of the arena labelled `label`, per residency
/// dump, in order.
fn arena_bytes(stderr: &str, label: &str) -> Vec<u64> {
    let tag = format!("label={label}]");
    stderr
        .lines()
        .filter(|l| l.contains(&tag))
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("bytes="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries bytes=N")
        })
        .collect()
}

const TYPES: &str = r#"
    type In { s: String = ""; b: Bytes = b""; }
    type Out { k: Int = 0; tag: String = ""; inner: In = In { }; }
    type Deep { k: Int = 0; mid: Out = Out { }; note: String = ""; }
    type BC { k: Int = 0; b: Bytes = b""; }
    type Pair { k: Int = 0; p: (String, Bytes) = ("", b""); }
    type Pt { x: Int = 0; y: Int = 0; }
    type Sc { k: Int = 0; p: Pt = Pt { }; }
    type SK { name: String = ""; inner: In = In { }; }
    type Lv { k: Int = 0; levels: [In; 4] = [In { }; 4]; }
    type TA { k: Int = 0; pair: ([Int; 2], String) = ([0, 0], ""); }
    type MA { k: Int = 0; m: [[Int; 2]; 2] = [[0, 0], [0, 0]]; tag: String = ""; }
    type IA { k: Int = 0; xs: [Int; 4] = [0, 0, 0, 0]; tag: String = ""; }
"#;

/// `map_decl` declares a `@form(hashmap)` locus named `M`; `body` is
/// one cycle of `H.step(x, b)` over it, with `self.n` counting cycles.
fn assert_map_flat(name: &str, map_decl: &str, body: &str) {
    let src = format!(
        r#"
        {TYPES}
        {map_decl}
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
    let bin = harness::unique_bin(&format!("hale_hashmap_nested_retire_{name}"));
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
    assert!(dumps[0] > 0, "{name}: the map's arena holds nothing; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: the map's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

const MAP: &str = "@form(hashmap) locus M { capacity { pool entries of";

#[test]
fn set_frees_a_replaced_nested_record() {
    assert_map_flat(
        "nested",
        &format!("{MAP} Out indexed_by k; }} }}"),
        "self.m.set(Out { k: self.n % 4, tag: x, inner: In { s: x, b: b } });",
    );
}

#[test]
fn set_frees_a_replaced_nested_nested_record() {
    assert_map_flat(
        "deep",
        &format!("{MAP} Deep indexed_by k; }} }}"),
        "self.m.set(Deep { k: self.n % 4, mid: Out { k: 1, tag: x, inner: In { s: x, b: b } }, note: x });",
    );
}

#[test]
fn set_frees_a_replaced_bytes_field() {
    assert_map_flat(
        "bytes",
        &format!("{MAP} BC indexed_by k; }} }}"),
        "self.m.set(BC { k: self.n % 4, b: b });",
    );
}

#[test]
fn set_frees_a_replaced_tuple_field() {
    assert_map_flat(
        "tuple",
        &format!("{MAP} Pair indexed_by k; }} }}"),
        "self.m.set(Pair { k: self.n % 4, p: (x, b) });",
    );
}

#[test]
fn set_frees_a_replaced_array_of_records() {
    assert_map_flat(
        "array",
        &format!("{MAP} Lv indexed_by k; }} }}"),
        "self.m.set(Lv { k: self.n % 4, levels: [In { s: x, b: b }; 4] });",
    );
}

/// A scalar array is a block of its own inside a tuple (the cell's
/// tuple field points at a block holding the array's pointer).
#[test]
fn set_frees_a_scalar_array_inside_a_tuple_field() {
    assert_map_flat(
        "tuparr",
        &format!("{MAP} TA indexed_by k; }} }}"),
        "self.m.set(TA { k: self.n % 4, pair: ([self.n, 2], x) });",
    );
}

/// An array of arrays: the outer array is a block, and so is each inner
/// one.
#[test]
fn set_frees_the_inner_arrays_of_an_array_field() {
    assert_map_flat(
        "mat",
        &format!("{MAP} MA indexed_by k; }} }}"),
        "self.m.set(MA { k: self.n % 4, m: [[self.n, 2], [3, 4]], tag: x });",
    );
}

/// A record's scalar fixed array is the cell's own inline bytes, not a
/// block: nothing to free, nothing to copy, and the cell reads back
/// whole after churn.
#[test]
fn a_records_inline_scalar_array_stays_inline() {
    assert_map_flat(
        "inline_arr",
        &format!("{MAP} IA indexed_by k; }} }}"),
        "self.m.set(IA { k: self.n % 4, xs: [self.n, 1, 2, 3], tag: x }); let g = self.m.get(self.n % 4) or IA { }; if g.xs[0] != self.n { std::process::exit(3); }",
    );
}

/// A nested record of scalars only: the set used to skip the anchor
/// walk for it and store the caller's scratch pointer (see
/// `a_nested_record_of_scalars_outlives_its_method`); now it is copied
/// into the map's arena, and freed when replaced.
#[test]
fn set_frees_a_replaced_nested_record_of_scalars() {
    assert_map_flat(
        "scalar",
        &format!("{MAP} Sc indexed_by k; }} }}"),
        "self.m.set(Sc { k: self.n % 4, p: Pt { x: self.n, y: 2 } });",
    );
}

#[test]
fn set_frees_a_replaced_nested_record_under_a_string_key() {
    assert_map_flat(
        "skey",
        &format!("{MAP} SK indexed_by name; }} }}"),
        r#"self.m.set(SK { name: "key-" + to_string(self.n % 4), inner: In { s: x, b: b } });"#,
    );
}

#[test]
fn remove_frees_a_nested_record() {
    assert_map_flat(
        "remove",
        &format!("{MAP} Out indexed_by k; }} }}"),
        "self.m.set(Out { k: self.n % 4, tag: x, inner: In { s: x, b: b } }); self.m.remove(self.n % 4) or discard;",
    );
}

/// The nested block carried from the key's own cell is kept, not
/// copied: get-modify-set allocates nothing.
#[test]
fn get_modify_set_keeps_the_nested_record() {
    assert_map_flat(
        "rmw",
        &format!("{MAP} Out indexed_by k; }} }}"),
        "let e = self.m.get(1) or Out { k: 1, tag: x, inner: In { s: x, b: b } }; self.m.set(Out { k: 1, tag: e.tag, inner: e.inner });",
    );
}

/// A nested record read from one cell and stored under another key is
/// copied, so each cell frees its own.
#[test]
fn a_record_stored_under_another_key_is_copied_and_freed() {
    assert_map_flat(
        "share",
        &format!("{MAP} Out indexed_by k; }} }}"),
        "self.m.set(Out { k: self.n % 4, tag: x, inner: In { s: x, b: b } }); let a = self.m.get(self.n % 4) or Out { }; self.m.set(Out { k: 10, tag: a.tag, inner: a.inner });",
    );
}

#[test]
fn a_serialized_map_frees_a_replaced_nested_record() {
    assert_map_flat(
        "serialized",
        "@form(hashmap, sync = serialized) locus M { capacity { pool entries of Out indexed_by k; } }",
        "self.m.set(Out { k: self.n % 4, tag: x, inner: In { s: x, b: b } }); let g = self.m.get(self.n % 4) or Out { };",
    );
}

/// A cell whose only heap part was a nested record of scalars skipped
/// the set's anchor walk and kept the pointer it was handed — into the
/// setting method's scratch, freed when that method returned. A later
/// `get` read whatever reused the block.
#[test]
fn a_nested_record_of_scalars_outlives_its_method() {
    let src = r#"
        type Pt { x: Int = 0; y: Int = 0; }
        type Sc { k: Int = 0; p: Pt = Pt { }; }
        @form(hashmap) locus M { capacity { pool entries of Sc indexed_by k; } }
        locus H {
            params { m: M = M { }; }
            fn put(i: Int) { self.m.set(Sc { k: i, p: Pt { x: i * 10, y: i * 100 } }); }
            fn junk(i: Int) -> Int {
                let q = Pt { x: 7777, y: 8888 };
                let r = Pt { x: q.x + i, y: 1 };
                return r.x + r.y;
            }
            fn show(i: Int) -> String {
                let c = self.m.get(i) or Sc { };
                return to_string(c.p.x) + "," + to_string(c.p.y);
            }
        }
        fn main() {
            let h = H { };
            h.put(1);
            h.put(2);
            let mut s = 0;
            let mut i = 0;
            while i < 1000 { s = s + h.junk(i); i = i + 1; }
            println("one=" + h.show(1) + " two=" + h.show(2));
        }
    "#;
    let bin = harness::unique_bin("hale_hashmap_scalar_nested");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert!(stdout.contains("one=10,100 two=20,200"), "stdout: {stdout}; stderr: {stderr}");
}

/// Under ASan, with the arena's chunk pool off: values read out of a
/// cell, shared into another key, carried over by get-modify-set, held
/// across a replace of their cell, stored into a field, and read from a
/// serialized map stay intact while the maps free what they replace.
/// The output is what a build freeing no nested block prints.
#[test]
fn nested_values_outlive_their_cells() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Out { k: Int = 0; tag: String = ""; inner: In = In { }; }
        type Pt { x: Int = 0; y: Int = 0; }
        type Sc { k: Int = 0; p: Pt = Pt { }; }
        @form(hashmap) locus M { capacity { pool entries of Out indexed_by k; } }
        @form(hashmap, sync = serialized) locus SM { capacity { pool entries of Out indexed_by k; } }
        @form(hashmap) locus PM { capacity { pool entries of Sc indexed_by k; } }
        locus H {
            params { m: M = M { }; sm: SM = SM { }; pm: PM = PM { }; kept: In = In { }; seen: Int = 0; }
            fn make(i: Int, x: Bytes) -> Out {
                return Out { k: i % 8, tag: "t" + to_string(i), inner: In { s: "s" + to_string(i), b: x } };
            }
            fn churn(i: Int, x: Bytes) {
                self.m.set(self.make(i, x));
                let a = self.m.get(i % 8) or Out { };
                self.m.set(Out { k: 100 + i % 3, tag: a.tag, inner: a.inner });
                let e = self.m.get(i % 8) or Out { };
                self.m.set(Out { k: e.k, tag: "r" + to_string(i), inner: e.inner });
                let g = self.m.get((i + 1) % 8) or Out { };
                self.m.set(self.make(i + 1, x));
                self.seen = self.seen + len(g.inner.s) + len(g.inner.b);
                self.kept = g.inner;
                if i % 5 == 0 { self.m.remove(100 + i % 3) or discard; }
                self.sm.set(self.make(i, x));
                let sg = self.sm.get(i % 8) or Out { };
                self.sm.set(self.make(i + 8, x));
                self.seen = self.seen + len(sg.inner.s) + len(sg.inner.b);
                self.pm.set(Sc { k: i % 4, p: Pt { x: i, y: i * 2 } });
            }
            fn report() -> String {
                let mut out = "";
                let mut k = 100;
                while k < 103 {
                    let v = self.m.get(k) or Out { tag: "none" };
                    out = out + v.tag + "/" + v.inner.s + " ";
                    k = k + 1;
                }
                let z = self.m.get(0) or Out { };
                let s = self.sm.get(3) or Out { };
                let p = self.pm.get(1) or Sc { };
                return out + z.tag + "/" + z.inner.s + " " + s.inner.s + " kept=" + self.kept.s
                    + " p=" + to_string(p.p.x) + "," + to_string(p.p.y) + " seen=" + to_string(self.seen);
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            while i < 3000 { h.churn(i, x); i = i + 1; }
            println(h.report());
        }
    "#;
    let bin = harness::unique_bin("hale_hashmap_nested_outlive");
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
            "t2997/s2997 t2998/s2998 t2999/s2999 t3000/s3000 s3003 kept=s2992 p=2997,5994 seen=105654"
        ),
        "stdout: {stdout}; stderr: {stderr}"
    );
}

/// The array side of `nested_values_outlive_their_cells`: a scalar
/// array inside a tuple field and the inner arrays of an array field,
/// whose blocks the maps now free, while values read out of them — held
/// across a replace, stored into a field, read from a serialized map —
/// stay intact under ASan. The pinned output is what the churn computes
/// (worked out independently by simulating it).
#[test]
fn array_values_outlive_their_cells() {
    let src = r#"
        type TA { k: Int = 0; pair: ([Int; 2], String) = ([0, 0], ""); }
        type MA { k: Int = 0; m: [[Int; 2]; 2] = [[0, 0], [0, 0]]; tag: String = ""; }
        @form(hashmap) locus MT { capacity { pool entries of TA indexed_by k; } }
        @form(hashmap) locus MM { capacity { pool entries of MA indexed_by k; } }
        @form(hashmap, sync = serialized) locus SM { capacity { pool entries of MA indexed_by k; } }
        locus H {
            params { mt: MT = MT { }; mm: MM = MM { }; sm: SM = SM { }; lt: TA = TA { }; lm: MA = MA { }; seen: Int = 0; }
            fn churn(i: Int) {
                self.mt.set(TA { k: i % 4, pair: ([i, i + 1], "t" + to_string(i)) });
                let g = self.mt.get((i + 1) % 4) or TA { };
                self.mt.set(TA { k: (i + 1) % 4, pair: ([i + 7, i + 8], "s" + to_string(i)) });
                self.seen = self.seen + g.pair.0[0] + g.pair.0[1] + len(g.pair.1);
                self.lt = g;
                self.mm.set(MA { k: i % 4, m: [[i, i * 2], [i * 3, i * 4]], tag: "m" + to_string(i) });
                let h = self.mm.get(i % 4) or MA { };
                self.mm.set(MA { k: 10 + i % 2, m: h.m, tag: h.tag });
                self.mm.set(MA { k: i % 4, m: [[i + 5, i + 6], [i + 7, i + 8]], tag: "r" + to_string(i) });
                self.seen = self.seen + h.m[1][1];
                self.lm = h;
                self.sm.set(MA { k: i % 4, m: [[i, 1], [2, 3]], tag: "q" });
                let q = self.sm.get(i % 4) or MA { };
                self.sm.set(MA { k: i % 4, m: [[0, 0], [0, 0]], tag: "z" });
                self.seen = self.seen + q.m[0][0];
            }
        }
        fn main() {
            let h = H { };
            let mut i = 0;
            while i < 2000 { h.churn(i); i = i + 1; }
            let a = h.mm.get(10) or MA { };
            let b = h.mm.get(11) or MA { };
            println("lt=" + to_string(h.lt.pair.0[0]) + "," + h.lt.pair.1 + " lm=" + to_string(h.lm.m[1][0]) + "," + h.lm.tag
                + " a=" + to_string(a.m[1][1]) + "," + a.tag + " b=" + to_string(b.m[1][1]) + "," + b.tag + " seen=" + to_string(h.seen));
        }
    "#;
    let bin = harness::unique_bin("hale_hashmap_array_outlive");
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
        stdout.contains("lt=1996,t1996 lm=5997,m1999 a=7992,m1998 b=7996,m1999 seen=13991884"),
        "stdout: {stdout}; stderr: {stderr}"
    );
}

/// A form's own fields live in the arena its cells' nested blocks are
/// retired from. A whole-record field store took the same-arena skip for
/// a nested block, so `self.saved = self.get(1)` in the map's own method
/// left the field sharing the cell's nested record: once the cell was
/// replaced and the owner's activation flushed, a later set reused the
/// block and the field read the new text. A vec form had the same hole
/// through `for x in self.items { self.saved = x; }`, and worse: the
/// walk rewrote the element it visited, so the element and the field
/// shared the fresh copy, which the element's replace then freed. The
/// field now takes its own deep copy, walked from a snapshot of the rhs.
/// Under ASan, after 200 replaces, every retained field still reads OLD.
#[test]
fn a_forms_own_field_keeps_what_it_was_given() {
    let src = r#"
        type Leaf { text: String = ""; }
        type Entry { key: Int = 0; leaf: Leaf = Leaf { }; }
        type Wrap { inner: Entry = Entry { }; t: (Leaf, Int) = (Leaf { }, 0); }
        @form(hashmap)
        locus Map {
            params { saved: Entry = Entry { }; leaf: Leaf = Leaf { }; w: Wrap = Wrap { }; again: Entry = Entry { }; }
            capacity { pool entries of Entry indexed_by key; }
            fn put(text: String) { self.set(Entry { key: 1, leaf: Leaf { text: text } }); }
            fn remember() {
                let row = self.get(1) or Entry { };
                self.saved = row;
                self.leaf = row.leaf;
                self.w = Wrap { inner: row, t: (row.leaf, 1) };
                self.again.leaf = row.leaf;
            }
            fn read() -> String {
                return self.saved.leaf.text + "/" + self.leaf.text + "/" + self.w.inner.leaf.text + "/"
                    + self.w.t.0.text + "/" + self.again.leaf.text;
            }
        }
        @form(vec)
        locus Vec {
            params { got: Entry = Entry { }; seen: Entry = Entry { }; }
            capacity { heap items of Entry; }
            fn put(text: String) {
                if self.len() == 0 { self.push(Entry { key: 1, leaf: Leaf { text: text } }); }
                else { self.set(0, Entry { key: 1, leaf: Leaf { text: text } }) or discard; }
            }
            fn remember() {
                let row = self.get(0) or Entry { };
                self.got = row;
                for x in self.items { self.seen = x; }
            }
            fn read() -> String { return self.got.leaf.text + "/" + self.seen.leaf.text; }
        }
        locus Holder {
            params { map: Map = Map { }; v: Vec = Vec { }; mine: Entry = Entry { }; }
            fn save() {
                self.map.put(std::str::upper("old"));
                self.map.remember();
                self.v.put(std::str::upper("old"));
                self.v.remember();
                let r = self.map.get(1) or Entry { };
                self.mine = r;
            }
            fn replace(text: String) { self.map.put(text); self.v.put(text); }
            fn read() -> String { return self.map.read() + " v=" + self.v.read() + " mine=" + self.mine.leaf.text; }
        }
        fn main() {
            let h = Holder { };
            h.save();
            let mut i = 0;
            while i < 200 { h.replace(std::str::upper("new" + to_string(i))); i = i + 1; }
            println(h.read());
        }
    "#;
    let bin = harness::unique_bin("hale_hashmap_form_field_keeps");
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
        stdout.contains("OLD/OLD/OLD/OLD/OLD v=OLD/OLD mine=OLD"),
        "stdout: {stdout}; stderr: {stderr}"
    );
}

/// The shape `sync = serialized` exists for, under ASan: a writer on
/// another pool replaces nested cells (retiring them at its own
/// activation boundaries) while the owner reads them. Every read is an
/// owned copy, taken under the map's lock, so the tag and the nested
/// String of one read always come from the same write.
#[test]
fn cross_pool_reads_of_nested_cells_are_owned_copies() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Out { k: Int = 0; tag: String = ""; inner: In = In { }; }

        @form(hashmap, sync = serialized)
        locus Table { capacity { pool entries of Out indexed_by k; } }

        locus Writer {
            params { t: Table = Table { }; }
            fn put(i: Int) {
                let n = to_string(i);
                self.t.set(Out { k: i % 16, tag: n, inner: In { s: n, b: std::bytes::from_string(n) } });
            }
            run() {
                let mut i = 0;
                while i < 4000 { self.put(i); i = i + 1; }
                println("writer done");
            }
        }

        main locus App {
            params { w: Writer = Writer { }; hits: Int = 0; bad: Int = 0; held: Out = Out { }; }
            placement { w: cooperative(pool = io); }

            fn probe(i: Int) {
                let r = self.w.t.get(i % 16) or { return; };
                if r.tag != r.inner.s { self.bad = self.bad + 1; }
                if r.tag != std::str::from_bytes(r.inner.b) { self.bad = self.bad + 1; }
                self.hits = self.hits + 1;
                if i == 7 { self.held = r; }
            }
            fn sweep() {
                let mut i = 0;
                while i < 2000 { self.probe(i); i = i + 1; }
            }
            run() {
                let mut round = 0;
                while round < 200 {
                    self.sweep();
                    if self.hits > 4000 { break; }
                    std::time::sleep(10ms);
                    round = round + 1;
                }
                std::time::sleep(200ms);
                print("hits_positive="); println(self.hits > 0);
                print("bad="); println(self.bad);
                print("held_whole="); println(self.held.tag == self.held.inner.s);
            }
        }
        fn main() { App { }; }
    "#;
    let bin = harness::unique_bin("hale_hashmap_nested_crosspool");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    for want in ["hits_positive=true", "bad=0", "held_whole=true", "writer done"] {
        assert!(stdout.contains(want), "missing {want}; stdout: {stdout}; stderr: {stderr}");
    }
}
