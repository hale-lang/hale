//! `@form(vec)` `clear()` and `truncate(n)` (friction sweep 2026-10-12, a
//! downstream handoff): a per-message scratch vec could not be hoisted
//! and reused — the only way to empty one was a `pop` loop, and
//! `bounded[T; N]` already had `clear`.
//!
//! Both drop elements the way `pop` does: each removed element, with
//! everything it owns (String and Bytes fields, nested records at any
//! depth), goes back on the vec's arena reuse list, and the buffer keeps
//! its capacity. Each churn shape runs 100 cycles, dumps arena
//! residency, runs to 100 000 and dumps again: the vec's arena must hold
//! the same bytes. The ASan program reads values taken out of the vec
//! before a clear or a truncate after it.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_form_vec_clear_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    stdout
}

/// What `clear` and `truncate` leave: the kept prefix, an empty vec
/// that takes pushes again, `truncate` at or past `len` (and below 0)
/// as documented, and a value read before the truncate intact.
#[test]
fn clear_and_truncate_semantics() {
    let out = run(
        "semantics",
        r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Msg { id: Int = 0; s: String = ""; inner: In = In { }; }
        @form(vec) locus V { capacity { heap xs of Msg; } }
        @form(vec) locus I { capacity { heap xs of Int; } }
        @form(vec) locus S { capacity { heap xs of String; } }
        fn main() {
            let v = V { };
            let i = I { };
            let s = S { };
            let mut k = 0;
            while k < 5 {
                v.push(Msg { id: k, s: "m" + to_string(k), inner: In { s: "i" + to_string(k) } });
                i.push(k);
                s.push("s" + to_string(k));
                k = k + 1;
            }
            let held = v.get(3) or Msg { };
            v.truncate(2);
            i.truncate(3);
            s.truncate(10);
            let s_len = s.len();
            s.truncate(-1);
            println(v.len(), " ", i.len(), " ", s_len, " ", s.len(), " ", held.s, held.inner.s);
            let a = v.get(1) or Msg { };
            let b = v.get(2) or Msg { id: -1 };
            let c = i.get(2) or -1;
            println(a.s, a.inner.s, " ", b.id, " ", c);
            v.clear();
            i.clear();
            println(v.len(), " ", i.len(), " ", v.is_empty());
            v.push(Msg { id: 9, s: "again" });
            i.push(7);
            let d = v.get(0) or Msg { };
            println(d.id, d.s, " ", i.get(0) or -1);
        }
        "#,
    );
    assert_eq!(out.trim(), "2 3 5 0 m3i3\nm1i1 -1 2\n0 0 true\n9again 7");
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
                .find_map(|w| w.strip_prefix("bytes="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries bytes=N")
        })
        .collect()
}

/// `vec_decl` declares a `@form(vec)` locus named `V`; `body` is one
/// cycle of `H.step(x, b)` over it. The vec's arena must hold the same
/// bytes after 100 cycles and after 100 000.
fn assert_vec_flat(name: &str, vec_decl: &str, body: &str) {
    let src = format!(
        r#"
        type In {{ s: String = ""; b: Bytes = b""; }}
        type Out {{ n: Int = 0; tag: String = ""; inner: In = In {{ }}; }}
        type Deep {{ id: Int = 0; mid: Out = Out {{ }}; note: String = ""; }}
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
    let bin = harness::unique_bin(&format!("hale_form_vec_clear_flat_{name}"));
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
    assert!(dumps[0] > 0, "{name}: the vec's arena holds nothing; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: the vec's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

/// The scratch-vec shape the handoff wanted: fill per message, clear.
#[test]
fn clear_frees_nested_records() {
    assert_vec_flat(
        "clear_deep",
        "@form(vec) locus V { capacity { heap items of Deep; } }",
        "let mut k = 0; \
         while k < 4 { self.v.push(Deep { id: k, mid: Out { n: k, tag: x, inner: In { s: x, b: b } }, note: x }); k = k + 1; } \
         self.v.clear();",
    );
}

#[test]
fn clear_frees_bare_strings() {
    assert_vec_flat(
        "clear_str",
        "@form(vec) locus V { capacity { heap items of String; } }",
        "self.v.push(x); self.v.push(x + \"!\"); self.v.clear();",
    );
}

#[test]
fn clear_frees_bare_bytes() {
    assert_vec_flat(
        "clear_bytes",
        "@form(vec) locus V { capacity { heap items of Bytes; } }",
        "self.v.push(b); self.v.push(b); self.v.push(b); self.v.clear();",
    );
}

/// `truncate` keeps a prefix and frees the rest: the vec refills to 5
/// and drops back to 2 every cycle.
#[test]
fn truncate_frees_the_dropped_suffix() {
    assert_vec_flat(
        "truncate_out",
        "@form(vec) locus V { capacity { heap items of Out; } }",
        "while self.v.len() < 5 { self.v.push(Out { n: self.v.len(), tag: x, inner: In { s: x, b: b } }); } \
         self.v.truncate(2);",
    );
}

/// Values read out of the vec — a `get` copy held in a field, a popped
/// value, an element of the kept prefix — stay whole across a `clear` or
/// a `truncate` that frees the elements they came from, under ASan.
#[test]
fn values_outlive_a_clear() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Out { n: Int = 0; tag: String = ""; inner: In = In { }; }
        @form(vec) locus Q { capacity { heap items of Out; } }
        @form(vec) locus QS { capacity { heap items of String; } }
        locus H {
            params { q: Q = Q { }; qs: QS = QS { }; kept: Out = Out { }; seen: Int = 0; }
            fn make(i: Int, x: Bytes) -> Out {
                return Out { n: i, tag: "t" + to_string(i), inner: In { s: "s" + to_string(i), b: x } };
            }
            fn cycle(i: Int, x: Bytes) {
                let mut k = 0;
                while k < 6 { self.q.push(self.make(i * 10 + k, x)); self.qs.push("q" + to_string(i * 10 + k)); k = k + 1; }
                let g = self.q.get(4) or Out { };
                self.kept = g;
                let p = self.q.pop() or Out { };
                if i % 2 == 0 { self.q.truncate(3); self.qs.truncate(2); } else { self.q.clear(); self.qs.clear(); }
                let first = self.q.get(0) or Out { tag: "-" };
                let fs = self.qs.get(0) or "-";
                self.seen = self.seen + len(g.inner.s) + len(p.tag) + len(p.inner.b) + len(first.tag) + len(fs);
                self.q.clear();
                self.qs.clear();
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            while i < 2000 { h.cycle(i, x); i = i + 1; }
            println("kept=" + h.kept.inner.s + "/" + h.kept.tag + " seen=" + to_string(h.seen));
        }
    "#;
    let bin = harness::unique_bin("hale_form_vec_clear_outlive");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert!(stdout.contains("kept=s19994/t19994 seen=60666"), "stdout: {stdout}; stderr: {stderr}");
}
