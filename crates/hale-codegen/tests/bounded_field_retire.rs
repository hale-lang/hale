//! A replaced `bounded[T; N]` field retires its elements' Strings,
//! Bytes and records.
//!
//! A record's `bounded[T; N]` field is inline storage, `{ i64 len,
//! [N x T] }`, not a pointer to a block, so the retire descriptor a
//! whole-value `self.f = v` store walks (`lotus_self_field_retire_replaced`)
//! had no entry for it: a replaced value's String, Bytes or record
//! elements stayed in the locus's arena for the locus's lifetime. A
//! record holding one 8 KB String in a `bounded[String; 4]`, stored
//! back over itself 20 000 times, peaked at 167 MB. `set` on an element
//! of such a field kept the element it replaced the same way, and so
//! did `clear` and `truncate` for the ones they dropped.
//!
//! The descriptor now has a bounded entry (`LOTUS_RETIRE_OFF_BOUNDED`):
//! the runtime reads the field's `len` and walks one slot's descriptor
//! over each live slot, `[0, len)` only — a slot past `len` can hold a
//! stale pointer `clear` left behind, which a walk would retire twice.
//! The elements a locus's bounded place stores are its own copies, so
//! no other owner shares a block the walk retires; a synced
//! `@form(hashmap)` whose cell holds such a field copies the elements
//! out on every read, as it does a cell's other blocks.
//!
//! The flat cases run 100 cycles, dump arena residency, run on to
//! 100 000 and dump again: the locus arena `H` must hold the same bytes
//! at both points. The alias cases hold every value read or copied out
//! of a replaced element to what it was given, under ASan with chunk
//! recycling off.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const TYPES: &str = r#"
    type Item { s: String = ""; n: Int = 0; }
    type Names { tag: Int = 0; items: bounded[String; 4]; }
    type Blobs { items: bounded[Bytes; 4]; }
    type Items { items: bounded[Item; 4]; }
    type Outer { n: Int = 0; inner: Names = Names { }; }
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

/// `fields` are H's params, `put` the body of `put(i: Int)`, `read` the
/// body of `read() -> String`, printed after the last `put` and held
/// to `want`.
fn assert_flat(name: &str, fields: &str, put: &str, read: &str, want: &str) {
    let src = format!(
        r#"
        {TYPES}
        locus H {{
            params {{ {fields} }}
            fn put(i: Int) {{ {put} }}
            fn read() -> String {{ {read} }}
        }}
        fn main() {{
            let h = H {{ }};
            let mut i = 0;
            while i < 100 {{ h.put(i); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.put(i); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
            println("read=" + h.read());
        }}
        "#
    );
    let bin = harness::unique_bin(&format!("hale_bounded_field_retire_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let dumps = h_arena_bytes(&stderr);
    assert!(dumps.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: H's arena grew between 100 and 100000 stores; stderr: {stderr}"
    );
    assert!(stdout.contains(&format!("read={want}\n")), "{name}: stdout: {stdout}");
}

#[test]
fn a_replaced_bounded_string_field_retires() {
    assert_flat(
        "string",
        "names: Names = Names { };",
        r#"let mut b = Names { tag: i };
           push(b.items, "a-" + to_string(i)) or discard;
           push(b.items, "b-" + to_string(i)) or discard;
           self.names = b;"#,
        r#"return to_string(count(self.names.items)) + "/" + (at(self.names.items, 0) or "") + "/"
               + (at(self.names.items, 1) or "");"#,
        "2/a-99999/b-99999",
    );
}

#[test]
fn a_value_stored_back_over_its_field_retires() {
    // The reported shape: the field read whole and stored back.
    assert_flat(
        "same",
        "names: Names = Names { };",
        r#"if i == 0 {
               let mut b = Names { };
               push(b.items, std::str::repeat("x", 800)) or discard;
               self.names = b;
           }
           let same = self.names;
           self.names = same;"#,
        r#"return to_string(count(self.names.items)) + "/" + to_string(len(at(self.names.items, 0) or ""));"#,
        "1/800",
    );
}

#[test]
fn a_replaced_bounded_bytes_field_retires() {
    assert_flat(
        "bytes",
        "blobs: Blobs = Blobs { };",
        r#"let mut b = Blobs { };
           push(b.items, std::bytes::from_string("blob-" + to_string(i))) or discard;
           self.blobs = b;"#,
        r#"return std::str::from_bytes(at(self.blobs.items, 0) or b"");"#,
        "blob-99999",
    );
}

#[test]
fn a_replaced_bounded_record_field_retires() {
    assert_flat(
        "record",
        "items: Items = Items { };",
        r#"let mut b = Items { };
           push(b.items, Item { s: "item-" + to_string(i), n: i }) or discard;
           push(b.items, Item { s: "other-" + to_string(i), n: i + 1 }) or discard;
           self.items = b;"#,
        r#"let a = at(self.items.items, 0) or Item { };
           let b = at(self.items.items, 1) or Item { };
           return a.s + "/" + to_string(a.n) + "/" + b.s + "/" + to_string(b.n);"#,
        "item-99999/99999/other-99999/100000",
    );
}

#[test]
fn a_bounded_field_inside_a_nested_record_retires() {
    // The bounded field one record down: the nested block's descriptor
    // carries the entry.
    assert_flat(
        "nested",
        "outer: Outer = Outer { };",
        r#"let mut b = Names { tag: i };
           push(b.items, "n-" + to_string(i)) or discard;
           self.outer = Outer { n: i, inner: b };"#,
        r#"return at(self.outer.inner.items, 0) or "";"#,
        "n-99999",
    )
}

#[test]
fn set_on_an_element_of_a_locus_field_retires_the_replaced_one() {
    assert_flat(
        "set_string",
        "names: Names = Names { };",
        r#"if i == 0 {
               push(self.names.items, "first") or discard;
               push(self.names.items, "second") or discard;
           }
           set(self.names.items, i % 2, "set-" + to_string(i)) or discard;"#,
        r#"return (at(self.names.items, 0) or "") + "/" + (at(self.names.items, 1) or "");"#,
        "set-99998/set-99999",
    );
}

#[test]
fn set_on_a_record_element_retires_its_block_and_string() {
    assert_flat(
        "set_record",
        "items: Items = Items { };",
        r#"if i == 0 { push(self.items.items, Item { }) or discard; }
           set(self.items.items, 0, Item { s: "set-" + to_string(i), n: i }) or discard;"#,
        r#"let a = at(self.items.items, 0) or Item { };
           return a.s + "/" + to_string(a.n);"#,
        "set-99999/99999",
    );
}

#[test]
fn set_on_a_locus_level_bounded_field_retires() {
    assert_flat(
        "set_locus_level",
        "xs: bounded[String; 2];",
        r#"if i == 0 { push(self.xs, "x") or discard; }
           set(self.xs, 0, "v-" + to_string(i)) or discard;"#,
        r#"return at(self.xs, 0) or "";"#,
        "v-99999",
    );
}

#[test]
fn clear_and_truncate_retire_what_they_drop() {
    assert_flat(
        "clear_truncate",
        "names: Names = Names { };",
        r#"clear(self.names.items);
           push(self.names.items, "c-" + to_string(i)) or discard;
           push(self.names.items, "d-" + to_string(i)) or discard;
           push(self.names.items, "e-" + to_string(i)) or discard;
           truncate(self.names.items, 2);"#,
        r#"return to_string(count(self.names.items)) + "/" + (at(self.names.items, 0) or "") + "/"
               + (at(self.names.items, 1) or "");"#,
        "2/c-99999/d-99999",
    );
}

/// Build `src` under ASan, run it with chunk recycling off, and return
/// its stdout, failing on a non-zero exit.
fn run_asan(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_bounded_field_retire_{name}"));
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    stdout
}

/// A slot past `len` keeps the pointer `clear` or `truncate` left in
/// it, which the shrink already retired. Every walk stops at `len`, so a
/// store over the field, a `set`, and another shrink never retire it a
/// second time — under ASan, with chunk recycling off, a double retire
/// hands one block to two owners and the readback goes wrong or ASan
/// reports.
#[test]
fn stale_slots_past_len_are_never_retired_twice() {
    let src = format!(
        r#"
        {TYPES}
        locus H {{
            params {{ names: Names = Names {{ }}; items: Items = Items {{ }}; }}
            fn put(i: Int) {{
                let t = std::str::upper("v" + to_string(i));
                clear(self.names.items);
                push(self.names.items, t + "a") or discard;
                push(self.names.items, t + "b") or discard;
                push(self.names.items, t + "c") or discard;
                truncate(self.names.items, 1);
                let same = self.names;
                self.names = same;
                set(self.names.items, 0, t + "s") or discard;
                let mut b = Items {{ }};
                push(b.items, Item {{ s: t + "x", n: i }}) or discard;
                push(b.items, Item {{ s: t + "y", n: i }}) or discard;
                clear(b.items);
                push(b.items, Item {{ s: t + "z", n: i }}) or discard;
                self.items = b;
                clear(self.items.items);
                push(self.items.items, Item {{ s: t + "w", n: i }}) or discard;
            }}
            fn read() -> String {{
                let it = at(self.items.items, 0) or Item {{ }};
                return to_string(count(self.names.items)) + "/" + (at(self.names.items, 0) or "") + "/"
                    + to_string(count(self.items.items)) + "/" + it.s;
            }}
        }}
        fn main() {{
            let h = H {{ }};
            let mut i = 0;
            while i < 300 {{ h.put(i); i = i + 1; }}
            println("read=" + h.read());
        }}
        "#
    );
    let stdout = run_asan("stale", &src);
    assert!(stdout.contains("read=1/V299s/1/V299w"), "stdout: {stdout}");
}

/// Every value read or copied out of a replaced bounded field keeps
/// what it was given: another field holding a copy of the record, a
/// locus String pushed into the field (the element is the field's own
/// copy, not the String's block), an element read before the replace
/// in the same activation, a `for` element held across a replace, a
/// `@form(vec)` element pushed from the field, and a String field
/// assigned from an element. An element read into a local is the
/// local's: writing into it leaves the field as it was.
#[test]
fn every_copy_of_a_replaced_element_survives() {
    let src = format!(
        r#"
        {TYPES}
        @form(vec)
        locus Saved {{ capacity {{ heap items of Names; }} }}
        locus H {{
            params {{
                title: String = std::str::upper("old-title");
                names: Names = Names {{ }};
                items: Items = Items {{ }};
                copy: Names = Names {{ }};
                first: String = "";
                v: Saved = Saved {{ }};
            }}
            fn put(text: String) {{
                let mut b = Names {{ }};
                push(b.items, text) or discard;
                self.names = b;
                let mut c = Items {{ }};
                push(c.items, Item {{ s: text, n: 1 }}) or discard;
                self.items = c;
            }}
            fn save() {{
                push(self.names.items, self.title) or discard;
                self.copy = self.names;
                self.first = at(self.names.items, 0) or "";
                self.v.push(self.names);
            }}
            fn swap(text: String) -> String {{
                let s = at(self.names.items, 0) or "";
                let it = at(self.items.items, 0) or Item {{ }};
                self.put(text);
                return s + "/" + it.s;
            }}
            fn walk(text: String) -> String {{
                let mut out = "";
                for s in self.names.items {{
                    self.put(text);
                    out = out + s;
                }}
                return out;
            }}
            fn write_local() -> String {{
                let mut it = at(self.items.items, 0) or Item {{ }};
                it.s = "local";
                let now = at(self.items.items, 0) or Item {{ }};
                return it.s + "/" + now.s;
            }}
            fn read() -> String {{
                let saved = self.v.get(0) or Names {{ }};
                return self.title + "/" + (at(self.copy.items, 0) or "") + "/" + (at(self.copy.items, 1) or "")
                    + "/" + self.first + "/" + (at(saved.items, 0) or "") + "/" + (at(saved.items, 1) or "");
            }}
        }}
        fn main() {{
            let h = H {{ }};
            h.put(std::str::upper("old"));
            h.save();
            println("swap=" + h.swap(std::str::upper("new")));
            println("walk=" + h.walk(std::str::upper("walked")));
            println("local=" + h.write_local());
            let mut i = 0;
            while i < 200 {{ h.put(std::str::upper("new" + to_string(i))); i = i + 1; }}
            println("read=" + h.read());
            println("now=" + (at(h.names.items, 0) or ""));
        }}
        "#
    );
    let stdout = run_asan("alias", &src);
    for want in [
        "swap=OLD/OLD",
        "walk=NEW\n",
        "local=local/WALKED",
        "read=OLD-TITLE/OLD/OLD-TITLE/OLD/OLD/OLD-TITLE",
        "now=NEW199",
    ] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}");
    }
}

/// A synced `@form(hashmap)` whose cell holds a bounded field of Strings
/// and records: the writer's replace retires the old cell's elements, so
/// every read path copies them out — `get`, `entry_at` and `for`. A
/// value read before 500 replaces of its key, and the values a sweep
/// read, still hold what they were given, under ASan with chunk
/// recycling off.
#[test]
fn a_synced_map_cell_with_a_bounded_field_copies_its_elements_out() {
    let src = r#"
        type Item { s: String = ""; n: Int = 0; }
        type Bal { acct: String; amts: bounded[String; 2]; its: bounded[Item; 2]; }

        @form(hashmap, sync = serialized)
        locus Table { capacity { pool entries of Bal indexed_by acct; } }

        main locus App {
            params { t: Table = Table { }; held: String = ""; held_item: String = ""; seen: Int = 0; }

            fn put(acct: String, v: String) {
                let mut b = Bal { acct: acct };
                push(b.amts, v) or discard;
                push(b.amts, v + "-2") or discard;
                push(b.its, Item { s: v + "-item", n: 1 }) or discard;
                self.t.set(b);
            }
            fn read_one() {
                let r = self.t.get("a1") or { return; };
                self.held = at(r.amts, 1) or "";
                let it = at(r.its, 0) or Item { };
                self.held_item = it.s;
            }
            fn hold_and_churn() -> String {
                let r = self.t.get("a1") or { return ""; };
                let e = self.t.entry_at(0) or { return ""; };
                let mut i = 0;
                while i < 50 { self.put("a1", std::str::upper("inner" + to_string(i))); i = i + 1; }
                let it = at(r.its, 0) or Item { };
                return (at(r.amts, 0) or "") + "/" + it.s + "/" + to_string(count(e.amts));
            }
            fn sweep() {
                let mut n = 0;
                for e in self.t.entries {
                    if (at(e.amts, 0) or "") != "" { n = n + 1; }
                }
                self.seen = n;
            }

            run() {
                self.put("a1", std::str::upper("original"));
                self.put("b2", std::str::upper("other"));
                self.read_one();
                let mut i = 0;
                while i < 500 { self.put("a1", std::str::upper("replacement-" + to_string(i))); i = i + 1; }
                self.sweep();
                println("held=" + self.held + "/" + self.held_item);
                println("inner=" + self.hold_and_churn());
                println("seen=" + to_string(self.seen));
                let cur = self.t.get("a1") or { return; };
                println("cur=" + (at(cur.amts, 0) or ""));
            }
        }
        fn main() { App { }; }
    "#;
    let stdout = run_asan("synced_map", src);
    for want in [
        "held=ORIGINAL-2/ORIGINAL-item",
        "inner=REPLACEMENT-499/REPLACEMENT-499-item/2",
        "seen=2",
        "cur=INNER49",
    ] {
        assert!(stdout.contains(want), "missing {want:?}; stdout: {stdout}");
    }
}

/// A `@form(vec)` element and a `sync = none` map cell holding a bounded
/// field retire its elements when replaced: the owner's arena holds the
/// same bytes after 100 and 100 000 replaces.
#[test]
fn a_form_cell_with_a_bounded_field_retires_its_elements() {
    let src = r#"
        type Item { s: String = ""; n: Int = 0; }
        type Row { k: Int = 0; names: bounded[String; 2]; its: bounded[Item; 2]; }
        type Cell { k: String; names: bounded[String; 2]; }
        @form(vec)
        locus H { capacity { heap items of Row; } }
        @form(hashmap)
        locus M { capacity { pool entries of Cell indexed_by k; } }
        locus App {
            params { v: H = H { }; m: M = M { }; }
            fn put(i: Int) {
                let mut r = Row { k: i };
                push(r.names, "row-" + to_string(i)) or discard;
                push(r.its, Item { s: "it-" + to_string(i), n: i }) or discard;
                if self.v.len() == 0 { self.v.push(r); } else { self.v.set(0, r) or discard; }
                let mut c = Cell { k: "key" };
                push(c.names, "cell-" + to_string(i)) or discard;
                self.m.set(c);
            }
            fn read() -> String {
                let r = self.v.get(0) or Row { };
                let it = at(r.its, 0) or Item { };
                let c = self.m.get("key") or Cell { k: "" };
                return (at(r.names, 0) or "") + "/" + it.s + "/" + (at(c.names, 0) or "");
            }
        }
        fn main() {
            let a = App { };
            let mut i = 0;
            while i < 100 { a.put(i); i = i + 1; }
            let _warm = std::process::dump_arena_residency();
            while i < 100000 { a.put(i); i = i + 1; }
            let _late = std::process::dump_arena_residency();
            println("read=" + a.read());
        }
    "#;
    let bin = harness::unique_bin("hale_bounded_field_retire_forms");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stderr: {stderr}", out.status);
    for label in ["H", "M"] {
        let tag = format!("label={label}]");
        let dumps: Vec<&str> = stderr.lines().filter(|l| l.contains(&tag)).collect();
        assert!(dumps.len() >= 2, "{label}: expected two residency dumps; stderr: {stderr}");
        let bytes = |l: &str| l.split_whitespace().find_map(|w| w.strip_prefix("bytes=")).map(str::to_string);
        assert_eq!(
            bytes(dumps[0]),
            bytes(dumps[1]),
            "{label}'s arena grew between 100 and 100000 replaces; stderr: {stderr}"
        );
    }
    assert!(stdout.contains("read=row-99999/it-99999/cell-99999"), "stdout: {stdout}");
}
