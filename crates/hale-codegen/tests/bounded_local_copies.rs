//! A local's `bounded[T; N]` list owns its elements (GH #1918).
//!
//! `push` and `set` on a list a locus holds store the place's own copy
//! of each pointer-shaped element (#1905). A record local's list
//! (`l.list`) kept the very block it was handed instead when that block
//! was already in the frame's arena, so the element and the binding it
//! came from were one block, and a write through either showed in the
//! other: `push(l.list, y); y.k = 2;` changed `l.list[0]`.
//!
//! Every bounded place now owns its elements. The owned-view column
//! decides, per `push` and `set` into a list no locus holds, whether the
//! slot may keep the value itself (`BoundedStore`): a fresh value (a
//! literal, a call's result) or a local not used after the store is
//! kept as it is; anything another name still reaches is copied. The
//! value-semantics cases run with chunk recycling off
//! (`LOTUS_NO_CHUNK_POOL=1`), so a block freed under a slot would read
//! as the junk it is under ASan rather than as the bytes it held; the
//! residency cases hold the kept shapes to no copy, measured as the
//! frame arena's growth over a loop.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build and run `src` with chunk recycling off; its stdout. `env`
/// adds variables for the child.
fn run(name: &str, src: &str, env: &[(&str, &str)]) -> (String, String) {
    let bin = harness::unique_bin(&format!("hale_bounded_local_copies_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut cmd = Command::new(&bin);
    cmd.env("LOTUS_NO_CHUNK_POOL", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    (stdout, stderr)
}

/// `stdout` is the lines of `want`, in any order (a locus's `run()`
/// prints beside `main`).
fn assert_lines(name: &str, stdout: &str, want: &[&str]) {
    let mut got: Vec<&str> = stdout.lines().collect();
    let mut want = want.to_vec();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want, "{name}: stdout: {stdout}");
}

#[test]
fn the_reported_program_keeps_both_values() {
    // The issue's program, as reported: it printed 2 and then 5.
    let src = r#"
        type In { k: Int = 0; }
        type L { list: bounded[In; 4]; }
        fn main() {
            let mut l = L { };
            let mut y = In { };
            push(l.list, y) or discard;
            y.k = 2;
            let a = at(l.list, 0) or In { };
            println(f"after y.k=2: list[0].k = {a.k}");
            let mut x = at(l.list, 0) or In { };
            x.k = 5;
            set(l.list, 0, x) or discard;
            println(f"after set: y.k = {y.k}");
        }
    "#;
    let (out, _) = run("reported", src, &[]);
    assert_lines("reported", &out, &["after y.k=2: list[0].k = 0", "after set: y.k = 2"]);
}

const TYPES: &str = r#"
    type In { k: Int = 0; s: String = ""; }
    type Out { n: Int = 0; inner: In = In { }; }
    type L { list: bounded[In; 4]; }
    type LO { list: bounded[Out; 4]; }
    type LS { list: bounded[String; 4]; }
    type LB { list: bounded[Bytes; 4]; }
    fn id(x: In) -> In { return x; }
"#;

#[test]
fn a_record_element_is_the_lists_own() {
    let src = format!(
        r#"{TYPES}
        fn main() {{
            // push of a local used after: the write stays in the local.
            let mut l = L {{ }};
            let mut y = In {{ k: 1, s: "one" }};
            push(l.list, y) or discard;
            y.k = 2;
            y.s = "two";
            let a = at(l.list, 0) or In {{ }};
            println(f"push {{a.k}} {{a.s}} {{y.k}}");
            // set of a local used after.
            let mut z = In {{ k: 3, s: "three" }};
            set(l.list, 0, z) or discard;
            z.k = 4;
            let b = at(l.list, 0) or In {{ }};
            println(f"set {{b.k}} {{b.s}} {{z.k}}");
            // the element read, written and set back leaves `z` alone; a
            // write with no set back leaves the element alone.
            let mut x = at(l.list, 0) or In {{ }};
            x.k = 30;
            set(l.list, 0, x) or discard;
            let mut w = at(l.list, 0) or In {{ }};
            w.k = 99;
            let c = at(l.list, 0) or In {{ }};
            println(f"back {{c.k}} {{z.k}}");
            // one local pushed twice: two blocks.
            let t = In {{ k: 7 }};
            push(l.list, t) or discard;
            push(l.list, t) or discard;
            let mut e = at(l.list, 1) or In {{ }};
            e.k = 70;
            set(l.list, 1, e) or discard;
            let f = at(l.list, 2) or In {{ }};
            println(f"twice {{f.k}} {{t.k}}");
            // through a fn that hands its parameter back.
            let mut p = In {{ }};
            push(l.list, id(p)) or discard;
            p.k = 6;
            let g = at(l.list, 3) or In {{ }};
            println(f"through {{g.k}}");
            // another list's element.
            let mut m = L {{ }};
            push(m.list, at(l.list, 0) or In {{ }}) or discard;
            let mut q = at(m.list, 0) or In {{ }};
            q.k = 77;
            set(m.list, 0, q) or discard;
            let r = at(l.list, 0) or In {{ }};
            println(f"other {{r.k}}");
            // a local pushed in a loop it was bound before.
            let mut v = In {{ }};
            let mut lv = L {{ }};
            for i in 0..3 {{ push(lv.list, v) or discard; }}
            v.k = 11;
            let s0 = at(lv.list, 0) or In {{ }};
            println(f"loop {{s0.k}}");
        }}
        "#
    );
    let (out, _) = run("record", &src, &[]);
    assert_lines(
        "record",
        &out,
        &["push 1 one 2", "set 3 three 4", "back 30 4", "twice 7 7", "through 0", "other 30", "loop 0"],
    );
}

#[test]
fn a_nested_record_element_is_copied_deeply() {
    let src = format!(
        r#"{TYPES}
        fn main() {{
            let mut lo = LO {{ }};
            let mut o = Out {{ n: 1 }};
            push(lo.list, o) or discard;
            o.inner.k = 3;
            o.inner.s = "x";
            let a = at(lo.list, 0) or Out {{ }};
            println(f"push {{a.inner.k}} '{{a.inner.s}}'");
            let mut p = Out {{ n: 2 }};
            p.inner.k = 5;
            set(lo.list, 0, p) or discard;
            p.inner.k = 6;
            let b = at(lo.list, 0) or Out {{ }};
            println(f"set {{b.inner.k}} {{p.inner.k}}");
            // a literal holding a local's block.
            let mut y = In {{ }};
            push(lo.list, Out {{ inner: y }}) or discard;
            y.k = 4;
            let c = at(lo.list, 1) or Out {{ }};
            println(f"literal {{c.inner.k}}");
            // a nested write through an element read and set back.
            let mut x = at(lo.list, 0) or Out {{ }};
            x.inner.k = 50;
            set(lo.list, 0, x) or discard;
            let d = at(lo.list, 0) or Out {{ }};
            println(f"back {{d.inner.k}} {{p.inner.k}}");
        }}
        "#
    );
    let (out, _) = run("nested", &src, &[]);
    assert_lines("nested", &out, &["push 0 ''", "set 5 6", "literal 0", "back 50 6"]);
}

#[test]
fn string_and_bytes_elements_keep_their_values() {
    let src = format!(
        r#"{TYPES}
        locus Held {{
            params {{ name: String = "abcd"; }}
            run() {{
                // A self field's String in `run()`, whose frame allocates
                // from the locus's arena: a same-length store overwrites
                // the field's bytes in place, which a shared element saw.
                let mut ls = LS {{ }};
                push(ls.list, self.name) or discard;
                self.name = "wxyz";
                push(ls.list, self.name) or discard;
                self.name = "lmno";
                println("held " + (at(ls.list, 0) or "") + " " + (at(ls.list, 1) or ""));
            }}
        }}
        fn main() {{
            let mut ls = LS {{ }};
            let mut s = "hello";
            push(ls.list, s) or discard;
            s = "world";
            set(ls.list, 0, s) or discard;
            push(ls.list, f"lit-{{s}}") or discard;
            s = "after";
            println("strings " + (at(ls.list, 0) or "") + " " + (at(ls.list, 1) or "") + " " + s);
            let mut lb = LB {{ }};
            let mut b = std::bytes::from_string("abc");
            push(lb.list, b) or discard;
            b = std::bytes::from_string("xyzw");
            set(lb.list, 0, b) or discard;
            push(lb.list, std::bytes::from_string("q")) or discard;
            b = std::bytes::from_string("");
            let b0 = at(lb.list, 0) or std::bytes::from_string("");
            let b1 = at(lb.list, 1) or std::bytes::from_string("");
            println(f"bytes {{len(b0)}} {{std::bytes::read_u8(b0, 0) or 0}} {{len(b1)}} {{len(b)}}");
            let h = Held {{ }};
        }}
        "#
    );
    let (out, _) = run("strings", &src, &[]);
    assert_lines(
        "strings",
        &out,
        &["strings world lit-world after", "bytes 4 120 1 0", "held abcd wxyz"],
    );
}

/// The `used` bytes of main's arena (`lotus.arena.global`), per
/// residency dump, in order.
fn global_used(stderr: &str) -> Vec<u64> {
    stderr
        .lines()
        .filter(|l| l.contains("label=lotus.arena.global]"))
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("used="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries used=N")
        })
        .collect()
}

#[test]
fn a_value_nothing_else_names_is_not_copied() {
    // Each loop runs 1000 times; between dumps, main's arena grows by
    // what the loop allocates. A fresh literal pushed or set costs the
    // literal alone, and so does a local not used after the push; a local
    // used after costs a copy more. The element read, written and set
    // back costs nothing (#1917 keeps the write in the element), and the
    // same write with no set back copies the element.
    let src = r#"
        type In { k: Int = 0; s: String = ""; }
        type L { list: bounded[In; 4]; }
        fn main() {
            let mut l = L { };
            push(l.list, In { }) or discard;
            let mut i = 0;
            let _0 = std::process::dump_arena_residency();
            while i < 1000 { clear(l.list); push(l.list, In { k: i }) or discard; i = i + 1; }
            let _1 = std::process::dump_arena_residency();
            while i < 2000 { set(l.list, 0, In { k: i }) or discard; i = i + 1; }
            let _2 = std::process::dump_arena_residency();
            while i < 3000 {
                clear(l.list);
                let mut y = In { k: i };
                y.k = y.k + 1;
                push(l.list, y) or discard;
                i = i + 1;
            }
            let _3 = std::process::dump_arena_residency();
            while i < 4000 {
                clear(l.list);
                let mut y = In { k: i };
                push(l.list, y) or discard;
                y.k = 0;
                i = i + 1;
            }
            let _4 = std::process::dump_arena_residency();
            while i < 5000 {
                let mut x = at(l.list, 0) or In { };
                x.k = i;
                set(l.list, 0, x) or discard;
                i = i + 1;
            }
            let _5 = std::process::dump_arena_residency();
            while i < 6000 {
                let mut x = at(l.list, 0) or In { };
                x.k = i;
                i = i + 1;
            }
            let _6 = std::process::dump_arena_residency();
            let last = at(l.list, 0) or In { };
            println(f"{count(l.list)} {last.k}");
        }
    "#;
    let (out, stderr) = run("residency", src, &[("LOTUS_ARENA_RESIDENCY", "1")]);
    assert!(out.starts_with("1 4999\n"), "stdout: {out}");
    let used = global_used(&stderr);
    assert!(used.len() >= 7, "expected seven residency dumps; stderr: {stderr}");
    let grew: Vec<u64> = used.windows(2).map(|w| w[1] - w[0]).collect();
    let (fresh_push, fresh_set, moved, copied, set_back, no_set) = (grew[0], grew[1], grew[2], grew[3], grew[4], grew[5]);
    // A copy of an `In` is at least 16 bytes a store, 16 000 a loop; a
    // dump and the first allocation's alignment move a count by a few
    // bytes, so two loops that cost the same agree to under 1 000.
    let same = |a: u64, b: u64| a.abs_diff(b) < 1000;
    assert!(fresh_push >= 16000, "the literals allocate: {grew:?}");
    assert!(same(fresh_set, fresh_push), "a fresh literal set is not copied: {grew:?}");
    assert!(same(moved, fresh_push), "a local not used after the push is not copied: {grew:?}");
    assert!(copied >= fresh_push + 16000, "a local used after the push is copied: {grew:?}");
    // The set back is #1917's slot store back (the list holds every
    // element alone): the write lands in the element and nothing is
    // copied; the same write with no set copies the element.
    assert!(set_back < 1000, "the set back of a written element copies nothing: {grew:?}");
    assert!(no_set >= 16000, "a written element with no set back is copied: {grew:?}");
}

/// Main's arena growth over `loop` run 1000 times on a local's list of
/// four `In`s filled with literals, after `setup`, then `after`.
fn set_back_cost(name: &str, setup: &str, body: &str, after: &str) -> (u64, String) {
    let src = format!(
        r#"
        type In {{ k: Int = 0; s: String = ""; }}
        type L {{ n: Int = 0; list: bounded[In; 4]; }}
        fn main() {{
            let mut l = L {{ }};
            for j in 0..4 {{ push(l.list, In {{ k: j, s: "s" }}) or discard; }}
            {setup}
            let mut i = 0;
            let _0 = std::process::dump_arena_residency();
            while i < 1000 {{ {body} i = i + 1; }}
            let _1 = std::process::dump_arena_residency();
            {after}
        }}
        "#
    );
    let (out, stderr) = run(name, &src, &[("LOTUS_ARENA_RESIDENCY", "1")]);
    let used = global_used(&stderr);
    assert!(used.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    (used[1] - used[0], out)
}

#[test]
fn a_slot_set_back_on_a_local_list_costs_at_most_the_writes_copy() {
    // The shape #1917 keeps in the element: the set adds no copy of its
    // own (`BoundedStore::Moved`), and the write none either.
    let read = r#"for j in 0..4 { let e = at(l.list, j) or In { }; print(f"{e.k} "); } println("");"#;
    let body = "let q = i % 4; let mut x = at(l.list, q) or In { }; x.k = x.k + 1; set(l.list, q, x) or discard;";
    let (kept, out) = set_back_cost("setback_kept", "", body, read);
    assert_eq!(out, "250 251 252 253 \n", "kept");
    assert!(kept < 1000, "the set back #1917 keeps copies nothing: {kept}");
    // The same loop on a list whose local is read whole (#1917 refuses):
    // the write copies the element, the set stores that copy as it is.
    let whole = "let c = l; println(count(c.list));";
    let (copied, out) = set_back_cost("setback_whole", whole, body, read);
    assert_eq!(out, "4\n250 251 252 253 \n", "whole");
    let no_set = "let q = i % 4; let mut x = at(l.list, q) or In { }; x.k = x.k + 1; l.n = x.k;";
    let (write_only, _) = set_back_cost("setback_write_only", whole, no_set, read);
    assert!(copied >= 16000, "the refused shape copies at the write: {copied}");
    assert!(copied.abs_diff(write_only) < 1000, "the set adds nothing to the write's copy: {copied} vs {write_only}");
}

#[test]
fn shapes_a_local_list_now_shares_with_no_one_keep_their_values() {
    // #1917 refused these before every `push` and `set` stored a value of
    // the list's own; now the set back keeps the write in the element,
    // and every other name keeps what it had.
    let src = r#"
        type In { k: Int = 0; s: String = ""; }
        type L { list: bounded[In; 4]; }
        type R { inner: In = In { }; }
        fn main() {
            // a local's value pushed in, and read after
            let mut l = L { };
            let mut y = In { k: 1 };
            push(l.list, y) or discard;
            let mut x = at(l.list, 0) or In { };
            x.k = 10;
            set(l.list, 0, x) or discard;
            let a = at(l.list, 0) or In { };
            println(f"pushed local: {a.k} {y.k}");
            // a record's field pushed in
            let mut l2 = L { };
            let r = R { inner: In { k: 2 } };
            push(l2.list, r.inner) or discard;
            let mut x2 = at(l2.list, 0) or In { };
            x2.k = 20;
            set(l2.list, 0, x2) or discard;
            let b = at(l2.list, 0) or In { };
            println(f"pushed field: {b.k} {r.inner.k}");
            // the element pushed into another list, directly
            let mut l3 = L { };
            push(l3.list, In { k: 3 }) or discard;
            let mut o = L { };
            push(o.list, at(l3.list, 0) or In { }) or discard;
            let mut x3 = at(l3.list, 0) or In { };
            x3.k = 30;
            set(l3.list, 0, x3) or discard;
            let c = at(l3.list, 0) or In { };
            let oc = at(o.list, 0) or In { };
            println(f"element into another: {c.k} {oc.k}");
            // through `x`, before and after the write
            let mut l4 = L { };
            push(l4.list, In { k: 4 }) or discard;
            let mut p = L { };
            let mut x4 = at(l4.list, 0) or In { };
            push(p.list, x4) or discard;
            x4.k = 40;
            push(p.list, x4) or discard;
            set(l4.list, 0, x4) or discard;
            let d = at(l4.list, 0) or In { };
            let p0 = at(p.list, 0) or In { };
            let p1 = at(p.list, 1) or In { };
            println(f"through x: {d.k} {p0.k} {p1.k}");
            // the other list's element written and set back afterwards
            let mut x5 = at(p.list, 1) or In { };
            x5.k = 50;
            set(p.list, 1, x5) or discard;
            let d2 = at(l4.list, 0) or In { };
            println(f"other list written: {d2.k}");
        }
    "#;
    let (out, _) = run("relaxed", src, &[]);
    assert_lines(
        "relaxed",
        &out,
        &["pushed local: 10 1", "pushed field: 20 2", "element into another: 30 3", "through x: 40 4 40", "other list written: 40"],
    );
}
