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
//! Each shape runs 100 cycles, dumps arena residency, runs to 100 000,
//! and dumps again: the vec's own arena (labelled with its locus name)
//! must hold the same bytes at both points. `a_popped_value_outlives_
//! its_slot` checks the other side under ASan: every popped value —
//! used at once, stored into a field, pushed straight back, returned —
//! stays intact after the vec has freed its block.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

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
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_vec_pop_retire_{name}"));
    build_executable(&program, &bin).expect("build");
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
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_vec_pop_outlives");
    harness::build_asan(&program, &bin);
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
