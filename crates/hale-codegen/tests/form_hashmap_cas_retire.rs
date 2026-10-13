//! `@form(hashmap, sync = striped)` and `sync = lockfree` reclaim what a
//! replaced or removed cell owns, as a plain or serialized map does.
//!
//! Neither installed a retire descriptor: their reads bracketed on the
//! rwlock or `lf_enter` and handed the cell's pointers out, so freeing a
//! replaced block could pull it from under a reader on another pool. A
//! churned cell's String, Bytes and nested records therefore stayed in
//! the map's arena for good — between 1 000 and 100 000 overwriting
//! sets of 4 keys, a cell holding a nested record carrying a String and
//! Bytes grew it 196 608 -> 16 056 320 bytes on either discipline, a
//! String cell 65 536 -> 4 128 768, a Bytes cell 65 536 -> 4 849 664,
//! and a String-keyed nested cell 131 072 -> 12 845 056.
//!
//! Now a read of a cell that owns blocks takes the cell's claim (the
//! COMMITTED -> CLAIMED CAS a writer's update takes) and copies the value
//! and everything it owns into the reader's arena before releasing it. A
//! writer's update and a lockfree remove are CASes from COMMITTED, so no
//! reader is inside the old blocks once one succeeds, and the writer
//! retires them inside its claim, deferred to its activation boundary. A
//! lockfree tombstone keeps its String key until the grow's migration
//! (a probe may still be comparing it), and a table grown by tombstones
//! alone is rebuilt at its own size instead of doubled.
//!
//! Each discipline's arena shape runs 100 cycles, dumps residency, runs
//! to 100 000 and dumps again: every map's arena must hold the same
//! bytes at both points. The ASan programs check the other side: what
//! was read out stays whole after its cell is replaced, removed or
//! tombstoned, on the reading thread and across pools.

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

/// The maps of one discipline: a String cell, a nested record carrying
/// a String and Bytes, a Bytes cell and a String-keyed nested cell, each
/// once replaced in place (`set`) and once set and removed (`*R`).
const MAPS: [&str; 8] = ["S", "N", "B", "K", "SR", "NR", "BR", "KR"];

fn assert_maps_flat(sync: &str) {
    let mut decls = String::new();
    for (name, cell, key) in [
        ("S", "SC", "k"),
        ("N", "NC", "k"),
        ("B", "BC", "k"),
        ("K", "SK", "name"),
        ("SR", "SC", "k"),
        ("NR", "NC", "k"),
        ("BR", "BC", "k"),
        ("KR", "SK", "name"),
    ] {
        decls.push_str(&format!(
            "@form(hashmap, sync = {sync}) locus {name} {{ capacity {{ pool entries of {cell} indexed_by {key}; }} }}\n"
        ));
    }
    let src = format!(
        r#"
        type In {{ s: String = ""; b: Bytes = b""; }}
        type SC {{ k: Int = 0; s: String = ""; }}
        type NC {{ k: Int = 0; tag: String = ""; inner: In = In {{ }}; }}
        type BC {{ k: Int = 0; b: Bytes = b""; }}
        type SK {{ name: String = ""; inner: In = In {{ }}; }}
        {decls}
        locus H {{
            params {{
                s: S = S {{ }}; n: N = N {{ }}; b: B = B {{ }}; k: K = K {{ }};
                sr: SR = SR {{ }}; nr: NR = NR {{ }}; br: BR = BR {{ }}; kr: KR = KR {{ }};
                c: Int = 0;
            }}
            fn step(x: String, y: Bytes) {{
                self.c = self.c + 1;
                let i = self.c % 4;
                let key = "key-" + to_string(i);
                self.s.set(SC {{ k: i, s: x }});
                self.n.set(NC {{ k: i, tag: x, inner: In {{ s: x, b: y }} }});
                self.b.set(BC {{ k: i, b: y }});
                self.k.set(SK {{ name: key, inner: In {{ s: x, b: y }} }});
                self.sr.set(SC {{ k: i, s: x }});
                self.sr.remove(i) or discard;
                self.nr.set(NC {{ k: i, tag: x, inner: In {{ s: x, b: y }} }});
                self.nr.remove(i) or discard;
                self.br.set(BC {{ k: i, b: y }});
                self.br.remove(i) or discard;
                self.kr.set(SK {{ name: key, inner: In {{ s: x, b: y }} }});
                self.kr.remove(key) or discard;
            }}
        }}
        fn main() {{
            let h = H {{ }};
            let x = std::str::upper("some string of forty characters or so...");
            let y = std::bytes::from_string(x);
            let mut i = 0;
            while i < 100 {{ h.step(x, y); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.step(x, y); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
        }}
        "#
    );
    let bin = harness::unique_bin(&format!("hale_hashmap_cas_retire_{sync}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{sync}: non-zero {:?}; stderr: {stderr}", out.status);
    for map in MAPS {
        let dumps = arena_bytes(&stderr, map);
        assert!(dumps.len() >= 2, "{sync} {map}: expected two residency dumps; stderr: {stderr}");
        assert!(dumps[0] > 0, "{sync} {map}: the map's arena holds nothing; stderr: {stderr}");
        assert_eq!(
            dumps[0], dumps[1],
            "{sync} {map}: the map's arena grew between 100 and 100000 cycles; stderr: {stderr}"
        );
    }
}

#[test]
fn a_striped_map_frees_what_its_replaced_and_removed_cells_owned() {
    assert_maps_flat("striped");
}

#[test]
fn a_lockfree_map_frees_what_its_replaced_and_removed_cells_owned() {
    assert_maps_flat("lockfree");
}

fn run_asan(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(name);
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    stdout
}

/// Under ASan, with the arena's chunk pool off: values read out of
/// striped and lockfree maps — by `get`, `entry_at`, `key_at` and a `for`
/// over the cells — before later calls replace, remove and tombstone
/// their cells, still read what they read. The output is what a build
/// freeing nothing prints.
#[test]
fn values_read_out_outlive_their_cells() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Out { k: Int = 0; tag: String = ""; inner: In = In { }; }
        type SK { name: String = ""; inner: In = In { }; }
        @form(hashmap, sync = striped) locus SM { capacity { pool entries of Out indexed_by k; } }
        @form(hashmap, sync = lockfree) locus LM { capacity { pool entries of Out indexed_by k; } }
        @form(hashmap, sync = striped) locus SS { capacity { pool entries of SK indexed_by name; } }
        @form(hashmap, sync = lockfree) locus LS { capacity { pool entries of SK indexed_by name; } }
        locus H {
            params {
                sm: SM = SM { }; lm: LM = LM { }; ss: SS = SS { }; ls: LS = LS { };
                a: Out = Out { }; b: Out = Out { }; c: SK = SK { }; d: SK = SK { };
                ea: Out = Out { }; eb: Out = Out { }; ka: String = ""; kb: String = ""; seen: Int = 0;
            }
            fn make(i: Int, x: Bytes) -> Out {
                return Out { k: i % 4, tag: "t" + to_string(i), inner: In { s: "s" + to_string(i), b: x } };
            }
            fn mk(i: Int, x: Bytes) -> SK {
                return SK { name: "key-" + to_string(i % 4), inner: In { s: "s" + to_string(i), b: x } };
            }
            fn put(i: Int, x: Bytes) {
                self.sm.set(self.make(i, x));
                self.lm.set(self.make(i, x));
                self.ss.set(self.mk(i, x));
                self.ls.set(self.mk(i, x));
            }
            fn hold() {
                self.a = self.sm.get(1) or Out { };
                self.b = self.lm.get(1) or Out { };
                self.c = self.ss.get("key-1") or SK { };
                self.d = self.ls.get("key-1") or SK { };
                self.ea = self.sm.entry_at(0) or Out { };
                self.eb = self.lm.entry_at(0) or Out { };
                self.ka = self.ss.key_at(0) or "";
                self.kb = self.ls.key_at(0) or "";
            }
            fn churn(i: Int, x: Bytes) {
                let g = self.sm.get(i % 4) or Out { };
                let h = self.lm.get(i % 4) or Out { };
                self.put(i, x);
                if i % 3 == 0 {
                    self.sm.remove(i % 4) or discard;
                    self.lm.remove(i % 4) or discard;
                    self.ss.remove("key-" + to_string(i % 4)) or discard;
                    self.ls.remove("key-" + to_string(i % 4)) or discard;
                }
                self.seen = self.seen + len(g.inner.s) + len(h.inner.b);
                for e in self.lm.entries { self.seen = self.seen + len(e.tag); }
                for e in self.ss.entries { self.seen = self.seen + len(e.name); }
            }
            fn report() -> String {
                return self.a.tag + "/" + self.a.inner.s + " " + self.b.tag + "/" + self.b.inner.s + " "
                    + self.c.name + "/" + self.c.inner.s + " " + self.d.name + "/" + std::str::from_bytes(self.d.inner.b) + " "
                    + self.ea.tag + " " + self.eb.tag + " " + self.ka + " " + self.kb + " seen=" + to_string(self.seen);
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            while i < 8 { h.put(i, x); i = i + 1; }
            h.hold();
            while i < 3000 { h.churn(i, x); i = i + 1; }
            println(h.report());
        }
    "#;
    let stdout = run_asan("hale_hashmap_cas_held", src);
    assert!(
        stdout.contains("t5/s5 t5/s5 key-1/s5 key-1/payload-bytes t4 t4 key-3 key-3 seen=112093"),
        "stdout: {stdout}"
    );
}

/// The shape these disciplines exist for, under ASan: a writer on another
/// pool replaces and removes nested cells of a striped and a lockfree map
/// (retiring them at its own activation boundaries) while the owner reads
/// them with `get`, `entry_at` and `key_at`. Every read is a whole, owned
/// copy, taken under the cell's claim: the tag and the nested String and
/// Bytes of one read always come from the same write, and a value held
/// past the writer's churn — a whole value stored in a field, or a got
/// value's nested record held in a local while the writer overwrites
/// its cell — still reads it. (A striped or lockfree read is deep, as a
/// serialized one is: the reader owns no view into a cell another pool
/// replaces.)
#[test]
fn cross_pool_reads_of_cas_maps_are_whole_owned_copies() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Out { k: Int = 0; tag: String = ""; inner: In = In { }; }
        type SK { name: String = ""; tag: String = ""; inner: In = In { }; }

        @form(hashmap, sync = striped)
        locus ST { capacity { pool entries of Out indexed_by k; } }
        @form(hashmap, sync = lockfree)
        locus LF { capacity { pool entries of SK indexed_by name; } }

        locus Writer {
            params { st: ST = ST { }; lf: LF = LF { }; }
            fn put(i: Int) {
                let n = to_string(i);
                self.st.set(Out { k: i % 16, tag: n, inner: In { s: n, b: std::bytes::from_string(n) } });
                self.lf.set(SK { name: "k" + to_string(i % 16), tag: n, inner: In { s: n, b: std::bytes::from_string(n) } });
                if i % 7 == 0 {
                    self.st.remove((i + 3) % 16) or discard;
                    self.lf.remove("k" + to_string((i + 3) % 16)) or discard;
                }
            }
            run() {
                let mut i = 0;
                while i < 6000 { self.put(i); i = i + 1; }
                println("writer done");
            }
        }

        main locus App {
            params { w: Writer = Writer { }; hits: Int = 0; bad: Int = 0; held: Out = Out { }; held2: SK = SK { }; hk: String = ""; }
            placement { w: cooperative(pool = io); }

            fn probe(i: Int) {
                let r = self.w.st.get(i % 16) or Out { tag: "x", inner: In { s: "x", b: b"x" } };
                if r.tag != r.inner.s { self.bad = self.bad + 1; }
                if r.tag != std::str::from_bytes(r.inner.b) { self.bad = self.bad + 1; }
                let q = self.w.lf.get("k" + to_string(i % 16)) or SK { tag: "x", inner: In { s: "x", b: b"x" } };
                if q.tag != q.inner.s { self.bad = self.bad + 1; }
                if q.tag != std::str::from_bytes(q.inner.b) { self.bad = self.bad + 1; }
                let e = self.w.st.entry_at(i % 4) or Out { tag: "x", inner: In { s: "x", b: b"x" } };
                if e.tag != e.inner.s { self.bad = self.bad + 1; }
                let k = self.w.lf.key_at(i % 4) or "k";
                if len(k) < 1 { self.bad = self.bad + 1; }
                self.hits = self.hits + 1;
                if i == 7 { self.held = r; self.held2 = q; self.hk = k; }
            }
            fn sweep() {
                let mut i = 0;
                while i < 1000 { self.probe(i); i = i + 1; }
            }
            run() {
                // A got value's nested record, held in a local across
                // the writer's overwrites of the same cell.
                self.sweep();
                let r0 = self.w.st.get(5) or Out { tag: "x", inner: In { s: "x", b: b"x" } };
                let q0 = self.w.lf.get("k5") or SK { tag: "x", inner: In { s: "x", b: b"x" } };
                let in0 = r0.inner;
                let in1 = q0.inner;
                let mut round = 0;
                while round < 200 {
                    self.sweep();
                    if self.hits > 6000 { break; }
                    std::time::sleep(5ms);
                    round = round + 1;
                }
                std::time::sleep(200ms);
                print("hits_positive="); println(self.hits > 0);
                print("bad="); println(self.bad);
                print("held_whole="); println(self.held.tag == self.held.inner.s && self.held2.tag == self.held2.inner.s && len(self.hk) > 0);
                print("held_nested="); println(in0.s == r0.tag && std::str::from_bytes(in0.b) == r0.tag
                    && in1.s == q0.tag && std::str::from_bytes(in1.b) == q0.tag);
            }
        }
        fn main() { App { }; }
    "#;
    let stdout = run_asan("hale_hashmap_cas_crosspool", src);
    for want in ["hits_positive=true", "bad=0", "held_whole=true", "held_nested=true", "writer done"] {
        assert!(stdout.contains(want), "missing {want}; stdout: {stdout}");
    }
}
