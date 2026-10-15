//! `@form(heap)`: a priority queue over a `heap` capacity slot, ordered
//! by a field of the cell record (`heap jobs of Job ordered_by min
//! priority;`). Branch-and-bound searches, Dijkstra and timer wheels
//! hand-rolled one over `@form(vec)` (friction sweep 2026-10-12, a
//! downstream handoff; forms are how Hale does collections).
//!
//! The ordering property runs a pseudo-random push/pop interleaving
//! and compares every popped key with a reference heap computed here;
//! the churn shapes hold the heap's arena flat from 100 to 100 000
//! cycles (a popped or cleared element is reclaimed as a vec's `pop`
//! reclaims it); the ASan program reads popped and peeked values after
//! the heap has freed what they came from.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_form_heap_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    stdout
}

/// The surface: a min heap of Int keys, a max heap of Float keys, a
/// String-keyed heap; `peek` leaves the root, `pop` on empty fails with
/// `EmptyError`, `clear` empties.
#[test]
fn heap_surface() {
    let out = run(
        "surface",
        r#"
        type Job { name: String = ""; priority: Int = 0; }
        type Ev { at: Float = 0.0; tag: String = ""; }
        type Word { w: String = ""; n: Int = 0; }
        @form(heap) locus Jobs { capacity { heap jobs of Job ordered_by min priority; } }
        @form(heap) locus Evs { capacity { heap evs of Ev ordered_by max at; } }
        @form(heap) locus Words { capacity { heap ws of Word ordered_by min w; } }
        fn main() {
            let q = Jobs { };
            let ps = [5, 3, 9, 1, 7, 3, 8, 2];
            for p in ps { q.push(Job { name: "j" + to_string(p), priority: p }); }
            let top = q.peek() or Job { name: "none" };
            println(top.name, " ", q.len());
            let mut out = "";
            while !q.is_empty() { let j = q.pop() or Job { }; out = out + j.name + ","; }
            let e = q.pop() or Job { name: "empty" };
            let pk = q.peek() or Job { name: "nothing" };
            println(out, " ", e.name, " ", pk.name);
            let ev = Evs { };
            ev.push(Ev { at: 1.5, tag: "a" });
            ev.push(Ev { at: 9.25, tag: "b" });
            ev.push(Ev { at: 3.0, tag: "c" });
            let x = ev.pop() or Ev { };
            let y = ev.peek() or Ev { };
            println(x.tag, y.tag, " ", ev.len());
            ev.clear();
            println(ev.len(), " ", ev.is_empty());
            let ws = Words { };
            ws.push(Word { w: "pear" });
            ws.push(Word { w: "apple" });
            ws.push(Word { w: "fig" });
            let a = ws.pop() or Word { };
            let b = ws.pop() or Word { };
            println(a.w, " ", b.w);
        }
        "#,
    );
    assert_eq!(out.trim(), "j1 8\nj1,j2,j3,j3,j5,j7,j8,j9, empty nothing\nbc 2\n0 true\napple fig");
}

/// The keys the program below pops, by a reference heap: `n` steps of
/// the same LCG, each pushing a key (`x % 500`) and, every third step,
/// popping; then the rest drained.
fn reference_ints(n: usize, max: bool) -> Vec<i64> {
    let mut x: i64 = 7;
    let mut out = Vec::new();
    let mut minh: BinaryHeap<Reverse<i64>> = BinaryHeap::new();
    let mut maxh: BinaryHeap<i64> = BinaryHeap::new();
    let pop = |minh: &mut BinaryHeap<Reverse<i64>>, maxh: &mut BinaryHeap<i64>, out: &mut Vec<i64>| {
        if max {
            if let Some(k) = maxh.pop() {
                out.push(k)
            }
        } else if let Some(Reverse(k)) = minh.pop() {
            out.push(k)
        }
    };
    for i in 0..n {
        x = (x * 1103515245 + 12345) % 2147483648;
        let k = x % 500;
        if max { maxh.push(k) } else { minh.push(Reverse(k)) }
        if i % 3 == 2 {
            pop(&mut minh, &mut maxh, &mut out);
        }
    }
    while !(if max { maxh.is_empty() } else { minh.is_empty() }) {
        pop(&mut minh, &mut maxh, &mut out);
    }
    out
}

fn ordering_program(dir: &str, key_ty: &str, key_expr: &str, show: &str) -> String {
    format!(
        r#"
        type Item {{ key: {key_ty} = {zero}; seq: Int = 0; note: String = ""; }}
        @form(heap) locus Q {{ capacity {{ heap items of Item ordered_by {dir} key; }} }}
        fn main() {{
            let q = Q {{ }};
            let mut x = 7;
            let mut i = 0;
            let mut out = "";
            while i < 3000 {{
                x = (x * 1103515245 + 12345) % 2147483648;
                q.push(Item {{ key: {key_expr}, seq: i, note: "n" + to_string(i) }});
                if i % 3 == 2 {{
                    let it = q.pop() or Item {{ }};
                    out = out + {show} + ",";
                }}
                i = i + 1;
            }}
            while !q.is_empty() {{
                let it = q.pop() or Item {{ }};
                out = out + {show} + ",";
            }}
            println(out);
        }}
        "#,
        zero = if key_ty == "String" { "\"\"" } else { "0" },
    )
}

fn joined<T: ToString>(keys: &[T]) -> String {
    keys.iter().map(|k| k.to_string() + ",").collect()
}

#[test]
fn a_min_heap_pops_keys_in_ascending_order() {
    let out = run("order_min", &ordering_program("min", "Int", "x % 500", "to_string(it.key)"));
    assert_eq!(out.trim(), joined(&reference_ints(3000, false)));
}

#[test]
fn a_max_heap_pops_keys_in_descending_order() {
    let out = run("order_max", &ordering_program("max", "Int", "x % 500", "to_string(it.key)"));
    assert_eq!(out.trim(), joined(&reference_ints(3000, true)));
}

/// String keys order by bytes: the reference sorts the same spellings.
#[test]
fn a_string_keyed_heap_pops_in_byte_order() {
    let out = run("order_str", &ordering_program("min", "String", "\"k\" + to_string(x % 997)", "it.key"));
    let mut x: i64 = 7;
    let mut out_ref: Vec<String> = Vec::new();
    let mut h: BinaryHeap<Reverse<String>> = BinaryHeap::new();
    for i in 0..3000 {
        x = (x * 1103515245 + 12345) % 2147483648;
        h.push(Reverse(format!("k{}", x % 997)));
        if i % 3 == 2 {
            out_ref.push(h.pop().unwrap().0);
        }
    }
    while let Some(Reverse(k)) = h.pop() {
        out_ref.push(k);
    }
    assert_eq!(out.trim(), joined(&out_ref));
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

/// `body` is one cycle of `H.step(x, b)` over `self.q`, a heap of `Job`
/// ordered by `pri`; `self.n` counts cycles. The heap's arena must hold
/// the same bytes after 100 cycles and after 100 000.
fn assert_heap_flat(name: &str, body: &str) {
    let src = format!(
        r#"
        type In {{ s: String = ""; b: Bytes = b""; }}
        type Job {{ pri: Int = 0; name: String = ""; data: Bytes = b""; inner: In = In {{ }}; }}
        @form(heap) locus Q {{ capacity {{ heap jobs of Job ordered_by min pri; }} }}
        locus H {{
            params {{ q: Q = Q {{ }}; n: Int = 0; }}
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
    let bin = harness::unique_bin(&format!("hale_form_heap_flat_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let dumps = arena_bytes(&stderr, "Q");
    assert!(dumps.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert!(dumps[0] > 0, "{name}: the heap's arena holds nothing; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: the heap's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

/// A queue at a steady depth: push one, pop one.
#[test]
fn push_pop_churn_stays_flat() {
    assert_heap_flat(
        "pushpop",
        "self.q.push(Job { pri: (self.n * 7919) % 101, name: x, data: b, inner: In { s: x, b: b } }); \
         if self.q.len() > 8 { let j = self.q.pop() or Job { }; }",
    );
}

#[test]
fn clear_churn_stays_flat() {
    assert_heap_flat(
        "clear",
        "let mut k = 0; \
         while k < 4 { self.q.push(Job { pri: k, name: x, data: b, inner: In { s: x, b: b } }); k = k + 1; } \
         let top = self.q.peek() or Job { }; \
         self.q.clear();",
    );
}

/// Values taken out of the heap — a popped job, a peeked one held in a
/// field, a popped job pushed straight back — stay whole while the heap
/// frees the elements they came from, under ASan.
#[test]
fn popped_and_peeked_values_outlive_their_slots() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Job { pri: Int = 0; name: String = ""; inner: In = In { }; }
        @form(heap) locus Q { capacity { heap jobs of Job ordered_by max pri; } }
        locus H {
            params { q: Q = Q { }; top: Job = Job { }; seen: Int = 0; }
            fn make(i: Int, x: Bytes) -> Job {
                return Job { pri: (i * 7919) % 10007, name: "j" + to_string(i), inner: In { s: "s" + to_string(i), b: x } };
            }
            fn step(i: Int, x: Bytes) {
                self.q.push(self.make(i, x));
                self.q.push(self.make(i + 1000, x));
                let p = self.q.peek() or Job { };
                self.top = p;
                let j = self.q.pop() or Job { };
                if i % 4 == 0 { self.q.push(j); }
                self.seen = self.seen + j.pri + len(j.name) + len(j.inner.s) + len(j.inner.b);
                if i % 50 == 49 { self.q.clear(); }
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            while i < 2000 { h.step(i, x); i = i + 1; }
            println("top=", h.top.name, "/", h.top.inner.s, " seen=", h.seen, " len=", h.q.len());
        }
    "#;
    let bin = harness::unique_bin("hale_form_heap_outlive");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert!(stdout.contains("top=j1999/s1999 seen=16073540 len=0"), "stdout: {stdout}; stderr: {stderr}");
}
