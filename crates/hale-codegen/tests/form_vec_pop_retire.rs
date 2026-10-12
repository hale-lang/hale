//! GH #1037 — `@form(vec)` frees what it removes or replaces.
//!
//! - `pop()` handed back the slot's own pointer and freed nothing, so a
//!   vec used as a queue (push, then pop once handled) grew by one
//!   payload per message for the owner's lifetime (~40 B a cycle for a
//!   bare String cell, ~62 B for a struct carrying one). It now hands
//!   the caller an owned copy — the clone-on-read `get` makes (GH #577)
//!   — and frees the element against it.
//! - `set(i, v)` over a struct cell freed the old element's String
//!   fields but not its Bytes ones (~47 B a set). The retire descriptor
//!   now lists Bytes fields too.
//!
//! - a field of record type (a nested record, a tuple) was skipped by
//!   the retire descriptor, so `set` and `pop` left the nested block
//!   and its own String / Bytes in the vec's arena (~260 B a replace in
//!   a downstream handoff's pending table). The descriptor now recurses
//!   into it, at any depth.
//!
//! Each shape runs 100 cycles, dumps arena residency, runs to 100 000,
//! and dumps again: the vec's own arena (labelled with its locus name)
//! must hold the same bytes at both points. `a_popped_value_outlives_
//! its_slot` checks the other side under ASan: every popped value —
//! used at once, stored into a field, pushed straight back, returned —
//! stays intact after the vec has freed its block.

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

/// `vec_decl` declares a `@form(vec)` locus named `V`; `body` is one
/// cycle of `H.step(x, b)` over it.
fn assert_vec_flat(name: &str, vec_decl: &str, body: &str) {
    let src = format!(
        r#"
        type S1 {{ s: String = ""; }}
        type B1 {{ b: Bytes = b""; }}
        type SB {{ n: Int = 0; s: String = ""; b: Bytes = b""; }}
        type In {{ s: String = ""; b: Bytes = b""; }}
        type Out {{ n: Int = 0; tag: String = ""; inner: In = In {{ }}; }}
        type Deep {{ id: Int = 0; mid: Out = Out {{ }}; note: String = ""; }}
        type Pair {{ p: (String, Bytes) = ("", b""); n: Int = 0; }}
        type TA {{ pair: ([Int; 2], String) = ([0, 0], ""); n: Int = 0; }}
        type MA {{ m: [[Int; 2]; 2] = [[0, 0], [0, 0]]; tag: String = ""; }}
        type SM {{ m: [[String; 2]; 2] = [["", ""], ["", ""]]; }}
        {vec_decl}
        locus H {{
            params {{ v: V = V {{ }}; }}
            fn step(x: String, b: Bytes) {{ {body} }}
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
    let bin = harness::unique_bin(&format!("hale_vec_pop_retire_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let dumps = arena_bytes(&stderr, "V");
    assert!(dumps.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: the vec's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

#[test]
fn pop_frees_a_struct_cell_with_a_string() {
    assert_vec_flat(
        "pop_s",
        "@form(vec) locus V { capacity { heap items of S1; } }",
        "self.v.push(S1 { s: x }); let g = self.v.pop() or S1 { };",
    );
}

#[test]
fn pop_frees_a_bare_string_cell() {
    assert_vec_flat(
        "pop_str",
        "@form(vec) locus V { capacity { heap items of String; } }",
        r#"self.v.push(x); let g = self.v.pop() or "";"#,
    );
}

#[test]
fn pop_frees_a_bare_bytes_cell() {
    assert_vec_flat(
        "pop_bytes",
        "@form(vec) locus V { capacity { heap items of Bytes; } }",
        r#"self.v.push(b); let g = self.v.pop() or b"";"#,
    );
}

#[test]
fn pop_frees_a_struct_cell_with_string_and_bytes() {
    assert_vec_flat(
        "pop_sb",
        "@form(vec) locus V { capacity { heap items of SB; } }",
        "self.v.push(SB { n: 1, s: x, b: b }); let g = self.v.pop() or SB { };",
    );
}

#[test]
fn set_frees_a_replaced_bytes_field() {
    assert_vec_flat(
        "set_b",
        "@form(vec) locus V { capacity { heap items of B1; } }",
        "if self.v.len() == 0 { self.v.push(B1 { b: b }); } self.v.set(0, B1 { b: b }) or discard;",
    );
}

#[test]
fn set_frees_a_replaced_nested_record() {
    assert_vec_flat(
        "set_nested",
        "@form(vec) locus V { capacity { heap items of Out; } }",
        "let o = Out { n: 1, tag: x, inner: In { s: x, b: b } }; if self.v.len() == 0 { self.v.push(o); } self.v.set(0, o) or discard;",
    );
}

#[test]
fn pop_frees_a_nested_record() {
    assert_vec_flat(
        "pop_nested",
        "@form(vec) locus V { capacity { heap items of Out; } }",
        "self.v.push(Out { n: 1, tag: x, inner: In { s: x, b: b } }); let g = self.v.pop() or Out { };",
    );
}

#[test]
fn set_frees_a_replaced_nested_nested_record() {
    assert_vec_flat(
        "set_deep",
        "@form(vec) locus V { capacity { heap items of Deep; } }",
        "let d = Deep { id: 2, mid: Out { n: 1, tag: x, inner: In { s: x, b: b } }, note: x }; if self.v.len() == 0 { self.v.push(d); } self.v.set(0, d) or discard;",
    );
}

#[test]
fn pop_frees_a_nested_nested_record() {
    assert_vec_flat(
        "pop_deep",
        "@form(vec) locus V { capacity { heap items of Deep; } }",
        "self.v.push(Deep { id: 2, mid: Out { n: 1, tag: x, inner: In { s: x, b: b } }, note: x }); let g = self.v.pop() or Deep { };",
    );
}

#[test]
fn set_frees_a_replaced_tuple_field() {
    assert_vec_flat(
        "set_tuple",
        "@form(vec) locus V { capacity { heap items of Pair; } }",
        "let q = Pair { p: (x, b), n: 3 }; if self.v.len() == 0 { self.v.push(q); } self.v.set(0, q) or discard;",
    );
}

// A scalar fixed array is inline only as a record's field; in a tuple or
// in another array it is a pointer to its own block, which the descriptor
// used to skip (the record-field guard applied at every depth).
#[test]
fn set_frees_a_scalar_array_inside_a_tuple_field() {
    assert_vec_flat(
        "set_tuparr",
        "@form(vec) locus V { capacity { heap items of TA; } }",
        "let q = TA { pair: ([1, 2], x), n: 3 }; if self.v.len() == 0 { self.v.push(q); } self.v.set(0, q) or discard;",
    );
}

#[test]
fn pop_frees_a_scalar_array_inside_a_tuple_field() {
    assert_vec_flat(
        "pop_tuparr",
        "@form(vec) locus V { capacity { heap items of TA; } }",
        "self.v.push(TA { pair: ([1, 2], x), n: 3 }); let g = self.v.pop() or TA { };",
    );
}

#[test]
fn set_frees_the_inner_arrays_of_an_array_field() {
    assert_vec_flat(
        "set_mat",
        "@form(vec) locus V { capacity { heap items of MA; } }",
        "let q = MA { m: [[1, 2], [3, 4]], tag: x }; if self.v.len() == 0 { self.v.push(q); } self.v.set(0, q) or discard;",
    );
}

#[test]
fn pop_frees_the_inner_arrays_of_an_array_field() {
    assert_vec_flat(
        "pop_mat",
        "@form(vec) locus V { capacity { heap items of MA; } }",
        "self.v.push(MA { m: [[1, 2], [3, 4]], tag: x }); let g = self.v.pop() or MA { };",
    );
}

#[test]
fn set_and_pop_free_an_array_of_string_arrays() {
    assert_vec_flat(
        "setpop_smat",
        "@form(vec) locus V { capacity { heap items of SM; } }",
        "let q = SM { m: [[x, x], [x, x]] }; self.v.push(q); self.v.set(0, q) or discard; let g = self.v.pop() or SM { };",
    );
}

#[test]
fn a_popped_value_outlives_its_slot() {
    let src = r#"
        type M { id: Int = 0; subj: String = ""; data: Bytes = b""; }
        @form(vec) locus Q { capacity { heap items of M; } }
        @form(vec) locus QB { capacity { heap items of Bytes; } }
        @form(vec) locus QS { capacity { heap items of String; } }
        locus H {
            params { q: Q = Q { }; qb: QB = QB { }; qs: QS = QS { }; last: M = M { }; lastb: Bytes = b""; seen: Int = 0; }
            fn enqueue(i: Int, x: Bytes) {
                self.q.push(M { id: i, subj: "subj." + to_string(i), data: x });
                self.qb.push(std::bytes::from_string("b" + to_string(i)));
                self.qs.push("s" + to_string(i));
            }
            fn take() -> String {
                let m = self.q.pop() or M { };
                let b = self.qb.pop() or b"";
                let s = self.qs.pop() or "";
                self.last = m;
                self.lastb = b;
                self.seen = self.seen + len(m.subj) + len(m.data) + len(b) + len(s);
                return m.subj + "|" + s;
            }
            fn roundtrip() {
                let m = self.q.pop() or M { };
                self.q.push(m);
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            let mut out = "";
            while i < 2000 {
                h.enqueue(i, x);
                h.enqueue(i + 1, x);
                h.roundtrip();
                out = h.take();
                if i % 50 == 49 {
                    while h.q.len() > 0 { let _t = h.take(); }
                }
                i = i + 1;
            }
            println("out=" + out + " last=" + h.last.subj + " lastb=" + std::str::from_bytes(h.lastb) + " seen=" + to_string(h.seen));
        }
    "#;
    let bin = harness::unique_bin("hale_vec_pop_outlives");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    // The same values the pre-fix build printed, when pop freed nothing.
    assert!(
        stdout.contains("out=subj.2000|s2000 last=subj.1950 lastb=b1950 seen=121349"),
        "stdout: {stdout}; stderr: {stderr}"
    );
}

/// The nested side of `a_popped_value_outlives_its_slot`: replaced and
/// popped elements whose nested records the vec now frees, while what was
/// read out of them — a popped value, a `get` copy stored in a field, a
/// value pushed straight back — stays intact, under ASan.
#[test]
fn nested_values_outlive_their_slots() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Out { n: Int = 0; tag: String = ""; inner: In = In { }; }
        type Deep { id: Int = 0; mid: Out = Out { }; note: String = ""; }
        @form(vec) locus Q { capacity { heap items of Deep; } }
        locus H {
            params { q: Q = Q { }; last: Deep = Deep { }; kept: Out = Out { }; seen: Int = 0; }
            fn make(i: Int, x: Bytes) -> Deep {
                return Deep { id: i, mid: Out { n: i, tag: "t" + to_string(i), inner: In { s: "s" + to_string(i), b: x } }, note: "n" + to_string(i) };
            }
            fn enqueue(i: Int, x: Bytes) {
                self.q.push(self.make(i, x));
                if self.q.len() > 1 {
                    let g = self.q.get(0) or Deep { };
                    self.kept = g.mid;
                    self.q.set(0, self.make(i + 1000, x)) or discard;
                }
            }
            fn take() -> String {
                let d = self.q.pop() or Deep { };
                self.last = d;
                self.seen = self.seen + len(d.note) + len(d.mid.tag) + len(d.mid.inner.s) + len(d.mid.inner.b);
                return d.mid.inner.s + "|" + d.note;
            }
            fn roundtrip() {
                let d = self.q.pop() or Deep { };
                self.q.push(d);
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            let mut out = "";
            while i < 2000 {
                h.enqueue(i, x);
                h.enqueue(i + 1, x);
                h.roundtrip();
                out = h.take();
                if i % 50 == 49 {
                    while h.q.len() > 0 { let _t = h.take(); }
                }
                i = i + 1;
            }
            println("out=" + out + " last=" + h.last.mid.inner.s + " kept=" + h.kept.inner.s + "/" + h.kept.tag + " seen=" + to_string(h.seen));
        }
    "#;
    let bin = harness::unique_bin("hale_vec_nested_outlives");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    // The values a build that freed no nested block prints: a reused
    // block overwritten under a live read would change them, which the
    // sanitizer alone cannot see (the vec frees onto its arena's reuse
    // list, not to the allocator).
    assert!(
        stdout.contains("out=s2000|n2000 last=s3000 kept=s2999/t2999 seen=105418"),
        "stdout: {stdout}; stderr: {stderr}"
    );
}

/// The array side of `nested_values_outlive_their_slots`: scalar arrays
/// inside a tuple field and inside an array field, whose blocks the vec
/// now frees on `set` and `pop`, while the values read out of them stay
/// intact, under ASan.
#[test]
fn array_blocks_outlive_their_slots() {
    let src = r#"
        type TA { pair: ([Int; 2], String) = ([0, 0], ""); n: Int = 0; }
        type MA { m: [[Int; 2]; 2] = [[0, 0], [0, 0]]; tag: String = ""; }
        @form(vec) locus QT { capacity { heap items of TA; } }
        @form(vec) locus QM { capacity { heap items of MA; } }
        locus H {
            params { qt: QT = QT { }; qm: QM = QM { }; lt: TA = TA { }; lm: MA = MA { }; seen: Int = 0; }
            fn enqueue(i: Int) {
                self.qt.push(TA { pair: ([i, i + 1], "t" + to_string(i)), n: i });
                self.qm.push(MA { m: [[i, i * 2], [i * 3, i * 4]], tag: "m" + to_string(i) });
                if self.qt.len() > 1 {
                    let g = self.qt.get(0) or TA { };
                    self.lt = g;
                    self.qt.set(0, TA { pair: ([i + 7, i + 8], "s" + to_string(i)), n: i + 1 }) or discard;
                    let h = self.qm.get(0) or MA { };
                    self.lm = h;
                    self.qm.set(0, MA { m: [[i + 5, i + 6], [i + 7, i + 8]], tag: "r" + to_string(i) }) or discard;
                }
            }
            fn take() -> Int {
                let t = self.qt.pop() or TA { };
                let m = self.qm.pop() or MA { };
                self.seen = self.seen + t.pair.0[0] + t.pair.0[1] + len(t.pair.1) + m.m[0][1] + m.m[1][0] + len(m.tag);
                return t.pair.0[1] + m.m[1][1];
            }
        }
        fn main() {
            let h = H { };
            let mut i = 0;
            let mut last = 0;
            while i < 2000 {
                h.enqueue(i);
                h.enqueue(i + 1);
                last = h.take();
                if i % 50 == 49 {
                    while h.qt.len() > 0 { let _t = h.take(); }
                }
                i = i + 1;
            }
            println("last=" + to_string(last) + " lt=" + to_string(h.lt.pair.0[0]) + "/" + h.lt.pair.1 + " lm=" + to_string(h.lm.m[1][1]) + "/" + h.lm.tag + " seen=" + to_string(h.seen));
        }
    "#;
    let bin = harness::unique_bin("hale_vec_array_blocks_outlive");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    // What a build that freed none of these blocks prints.
    assert!(
        stdout.contains("last=10001 lt=2006/s1999 lm=2007/r1999 seen=27931652"),
        "stdout: {stdout}; stderr: {stderr}"
    );
}
