//! A store into a locus's field from outside its methods is that locus's
//! own store (GH #1561; spec/types.md § A local bound from a place is
//! yours, spec/memory.md rules 6 and 7).
//!
//! `l.name = s` in a free fn with a handle parameter, `let l = L { };
//! l.name = s`, `h.loc.f = v` through a child, `ls[i].f = v` on an array
//! of loci and `self.loc.f = v` in the parent's method all lower like a
//! method of the field's locus storing `self.f = v`: the value is copied
//! into the arena of the LAST locus on the path (the one whose storage
//! holds the slot), the replaced value is retired, and the store ends the
//! way a method call does, flushing that arena's pending retires. Before,
//! the locus held the caller's own block: a later same-length overwrite
//! inside the locus rewrote the caller's immutable String, a write by
//! either side reached the other, nothing was retired (a long-lived locus
//! updated from free fns grew per store), and a copy made in the root's
//! arena was freed under a child that outlived its parent.
//!
//! Values are built at run time (`heap`), so a freed block shows as a
//! wrong value or a sanitizer report; the ownership program runs under
//! ASan with `LOTUS_NO_CHUNK_POOL=1` (GH #816).

use std::path::PathBuf;
use std::process::{Command, Output};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_run(name: &str, src: &str, options: hale_codegen::BuildOptions) -> Output {
    let bin = harness::unique_bin(&format!("hale_locus_field_store_{name}"));
    build_opts::build_source(src, &bin, &options).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

fn clean_stdout(name: &str, out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    for bad in ["ThreadSanitizer", "AddressSanitizer", "LeakSanitizer", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{name}: {bad} in:\n{stderr}");
    }
    assert!(out.status.success(), "{name}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    stdout
}

fn ir(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_locus_field_store_ir_{name}"));
    let ll: PathBuf = bin.with_extension("ll");
    let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let text = std::fs::read_to_string(&ll).unwrap_or_else(|e| panic!("{name}: read the IR: {e}"));
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    text
}

/// The body of the function whose symbol contains `name`.
fn body<'a>(ir: &'a str, name: &str) -> &'a str {
    let start = ir
        .match_indices("define ")
        .map(|(i, _)| i)
        .find(|&i| ir[i..].lines().next().is_some_and(|l| l.contains(name)))
        .unwrap_or_else(|| panic!("no function `{name}` in the IR"));
    let end = ir[start..].find("\n}\n").map(|e| start + e + 3).unwrap_or(ir.len());
    &ir[start..end]
}

const TYPES: &str = r#"
type Inner { k: Int = 0; s: String = ""; }
type In { k: Int = 0; s: String = ""; inner: Inner = Inner { }; }
type R { arr: [In; 2] = [In { }, In { }]; }
type Cell { s: String = ""; n: Int = 0; }
fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
fn fresh() -> R {
    return R { arr: [In { k: 1, s: heap("a0"), inner: Inner { k: 10, s: heap("i0") } }, In { k: 2, s: heap("a1") }] };
}
fn show(label: String, r: R) { println(label, ": a0=", r.arr[0].k, "/", r.arr[0].inner.k, " s=", r.arr[0].s); }
fn id_str(x: String) -> String { return x; }
fn churn() -> Int {
    let mut i = 0;
    let mut n = 0;
    while i < 300 { let x = fresh(); n = n + len(x.arr[0].inner.s); i = i + 1; }
    return n;
}
locus L {
    params { r: R = R { }; name: String = ""; c: Cell = Cell { }; ints: [Int; 3] = [0, 0, 0]; }
    fn bump() { self.r.arr[0].k = 9; self.r.arr[0].inner.k = 90; }
    fn rename() { self.name = "ZZZ"; }
    fn recell() { self.c = Cell { s: heap("other"), n: 0 }; }
    fn k0() -> Int { return self.r.arr[0].k; }
    fn nm() -> String { return self.name; }
    fn cs() -> String { return self.c.s; }
    fn i1() -> Int { return self.ints[1]; }
    fn set_c(x: Cell) { self.c = x; }
}
locus HL {
    params { loc: L = L { }; }
    fn store_child(s: String) { self.loc.name = s + "-child"; }
}
"#;

/// Every shape of a store from outside the locus: the locus and the
/// caller never share a block afterwards, in either direction, whatever
/// spelled the value (a place, an `if`, a call, a concatenation).
#[test]
fn a_store_from_outside_a_locus_never_shares_the_callers_value() {
    let src = format!(
        "{TYPES}
        fn put_arr(l: L, r: R) {{ l.r.arr = r.arr; }}
        fn put_name(l: L, s: String) {{ l.name = s; }}
        fn put_if(l: L, s: String, f: Bool) {{ l.name = if f {{ s }} else {{ \"xyz\" }}; }}
        fn put_call(l: L, s: String) {{ l.name = id_str(s); }}
        fn put_cell(l: L, c: Cell) {{ l.c = c; }}
        fn put_child(h: HL) {{ h.loc.name = heap(\"abc\") + heap(\"defgh\"); }}
        fn put_ints(l: L) {{ l.ints = [4, 5, 6]; l.ints[2] = 7; }}
        fn put_plus(l: L, s: String) {{ l.name = s; l.name += \"!\"; }}
        fn mkh(l: L) {{ let h = HL {{ loc: l }}; put_child(h); }}
        fn mkh_method(l: L) {{ let h = HL {{ loc: l }}; h.store_child(heap(\"own\")); }}
        fn main() {{
            let r = fresh();
            let l = L {{ }};
            put_arr(l, r);
            l.bump();
            show(\"locus wrote\", r);
            let q = fresh();
            let l2 = L {{ }};
            put_arr(l2, q);
            q.arr[0].k = 5;
            println(\"caller wrote: l2.k0=\", l2.k0());
            let s = heap(\"ab\");
            let l3 = L {{ }};
            put_name(l3, s);
            l3.rename();
            println(\"param: s=\", s, \" l=\", l3.nm());
            let l4 = L {{ }};
            put_if(l4, s, true);
            l4.rename();
            println(\"if: s=\", s, \" l=\", l4.nm());
            let l5 = L {{ }};
            put_call(l5, s);
            l5.rename();
            println(\"call: s=\", s, \" l=\", l5.nm());
            let l6 = L {{ }};
            l6.name = s;
            l6.rename();
            println(\"local handle: s=\", s, \" l=\", l6.nm());
            let ls = [L {{ }}, L {{ }}];
            let r2 = fresh();
            ls[0].r.arr = r2.arr;
            ls[0].bump();
            show(\"element\", r2);
            let c = Cell {{ s: heap(\"cell\"), n: 1 }};
            let l7 = L {{ }};
            put_cell(l7, c);
            l7.recell();
            println(\"record: c=\", c.s, \" l=\", l7.cs());
            let c2 = Cell {{ s: heap(\"meth\"), n: 1 }};
            let l8 = L {{ }};
            l8.set_c(c2);
            l8.recell();
            println(\"method record: c=\", c2.s, \" l=\", l8.cs());
            let kid = L {{ }};
            mkh(kid);
            println(churn());
            println(\"child: name=\", kid.nm());
            let kid2 = L {{ }};
            mkh_method(kid2);
            println(churn());
            println(\"self child: name=\", kid2.nm());
            let l9 = L {{ }};
            put_ints(l9);
            println(\"inline: i1=\", l9.i1());
            let l10 = L {{ }};
            put_plus(l10, s);
            println(\"plus: s=\", s, \" l=\", l10.nm());
        }}"
    );
    let out = build_run("shapes", &src, hale_codegen::BuildOptions { asan: true, ..build_opts::options() });
    let out = clean_stdout("shapes", &out);
    for want in [
        "locus wrote: a0=1/10 s=A02",
        "caller wrote: l2.k0=1",
        "param: s=AB2 l=ZZZ",
        "if: s=AB2 l=ZZZ",
        "call: s=AB2 l=ZZZ",
        "local handle: s=AB2 l=ZZZ",
        "element: a0=1/10 s=A02",
        "record: c=CELL4 l=OTHER5",
        "method record: c=METH4 l=OTHER5",
        "child: name=ABC3DEFGH5",
        "self child: name=OWN3-child",
        "inline: i1=5",
        "plus: s=AB2 l=AB2!",
    ] {
        assert!(out.contains(want), "missing `{want}` in:\n{out}");
    }
}

/// A free fn storing into a long-lived locus's String and record fields
/// retires what each store replaces and flushes the locus's arena, as a
/// method call would: 200k stores of varying length stay at the method
/// form's floor. Without the flush, the same loop (no method called on
/// the locus between stores) held every replaced block pending: 2M
/// stores reached 275 MB against 4.7 MB.
#[test]
fn a_store_from_outside_retires_what_it_replaces() {
    let prog = |call: &str| {
        format!(
            r#"
type Cell {{ s: String = ""; n: Int = 0; }}
locus L {{
    params {{ name: String = ""; c: Cell = Cell {{ }}; }}
    fn setname(s: String) {{ self.name = s; self.c = Cell {{ s: s, n: len(s) }}; }}
    fn total() -> Int {{ return len(self.name) + self.c.n; }}
}}
fn put(l: L, s: String) {{ l.name = s; l.c = Cell {{ s: s, n: len(s) }}; }}
locus Rec {{
    params {{ l: L = L {{ }}; n: Int = 200000; }}
    fn record(i: Int) {{ let s = "value." + std::str::repeat("x", i - (i / 50) * 50); {call} }}
    run() {{
        let mut i = 0;
        while i < self.n {{ self.record(i); i = i + 1; }}
        print("total="); println(self.l.total());
        print("rss_statm="); println(std::io::fs::read_file("/proc/self/statm") or "");
    }}
}}
fn main() {{ let r = Rec {{ }}; }}
"#
        )
    };
    let mb = |name: &str, src: &str| {
        let out = build_run(name, src, build_opts::options());
        let stdout = clean_stdout(name, &out);
        assert!(stdout.contains("total=110"), "{name}: {stdout}");
        let statm = stdout.lines().find_map(|l| l.strip_prefix("rss_statm=")).expect("statm line");
        harness::statm_resident_bytes(statm) / 1048576
    };
    let method = mb("retire_method", &prog("self.l.setname(s);"));
    let free = mb("retire_free_fn", &prog("put(self.l, s);"));
    let child = mb("retire_child", &prog("self.l.name = s; self.l.c = Cell { s: s, n: len(s) };"));
    assert!(method < 30, "the method form should recycle: {method}MB");
    assert!(
        free <= method + 4 && child <= method + 4,
        "200k stores into a locus's fields from outside its methods must stay at the \
         method form's floor: free fn {free}MB, child {child}MB, method {method}MB"
    );
}

/// The store copies once, into the locus: a free fn's `l.name = s` is the
/// owner's in-place String store and its flush, with no clone into the
/// caller's arena beside it (GH #1558 made one, before the store owned
/// the value).
#[test]
fn a_store_from_outside_copies_once_into_the_locus() {
    let src = r#"
        locus L { params { name: String = ""; } fn nm() -> String { return self.name; } }
        fn put(l: L, s: String) { l.name = s; }
        fn main() { let l = L { }; put(l, std::str::upper("ab")); println(l.nm()); }
    "#;
    let ir = ir("copies_once", src);
    let put = body(&ir, "put");
    assert!(put.contains("@lotus_str_assign_in_place"), "{put}");
    assert!(put.contains("@lotus_arena_flush_retired"), "{put}");
    assert!(!put.contains("@lotus_str_clone"), "a second copy in the caller's arena: {put}");
}

/// Two threads, each storing from free fns and through `self.child` into
/// loci of its own pool: the stores run on the thread their locus runs
/// on, so ThreadSanitizer sees no race. (A store into a locus on another
/// pool is refused at check: hale-types' `placement` tests.)
#[test]
fn same_pool_stores_from_two_threads_are_race_free_under_tsan() {
    let src = r#"
fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
type R { s: String = ""; k: Int = 0; }
locus C { params { name: String = ""; r: R = R { }; } fn nm() -> String { return self.name; } }
fn put(c: C, s: String) { c.name = s; c.r = R { s: s + "!", k: len(s) }; }
locus W {
    params { c: C = C { }; n: Int = 0; }
    run() {
        let mut i = 0;
        while i < 20000 { put(self.c, heap("w-" + to_string(i))); self.c.name = heap("x" + to_string(i)); i = i + 1; }
        self.n = len(self.c.nm());
    }
}
main locus App {
    params { w: W = W { }; c: C = C { }; }
    placement { w: cooperative(pool = p); }
    run() {
        let mut i = 0;
        while i < 20000 { put(self.c, heap("main-" + to_string(i))); i = i + 1; }
        println("done ", len(self.c.nm()) > 0);
    }
}
fn main() { App { }; }
"#;
    for round in 0..2 {
        let out = build_run("tsan", src, hale_codegen::BuildOptions { tsan: true, ..build_opts::options() });
        let stdout = clean_stdout(&format!("tsan round {round}"), &out);
        assert_eq!(stdout.trim(), "done true", "{stdout}");
    }
}

/// A locus-typed field replaced from outside its owner (`h.loc = L { }`
/// in a free fn, `hs[i].loc = L { }`) is WS1#4's lifecycle reassign, as
/// the owner's own `self.loc = L { }` is: the old child dissolves (once),
/// and the new one is built as the owner's field, adopted by its
/// lifecycle — alive after the replacing frame returns, dissolved once
/// with its owner. Before, the new child was a temporary the replacing
/// frame dissolved on return (the field held a torn-down locus) and the
/// old child was never dissolved.
#[test]
fn a_child_replaced_from_outside_its_owner_is_reclaimed_and_adopted() {
    let src = r#"
fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
locus L {
    params { name: String = ""; }
    birth() { println("birth ", self.name); }
    dissolve() { println("dissolve ", self.name); }
    fn nm() -> String { return self.name; }
}
locus HL {
    params { loc: L = L { name: heap("first") }; }
    fn renew() { self.loc = L { name: heap("method") }; }
    fn locname() -> String { return self.loc.nm(); }
}
fn replace(h: HL) { h.loc = L { name: heap("outside") }; }
fn churn() -> Int { let mut i = 0; let mut n = 0; while i < 300 { n = n + len(heap("zzzzzzzz" + to_string(i))); i = i + 1; } return n; }
fn main() {
    let h = HL { };
    replace(h);
    println(churn());
    println("after replace: ", h.locname());
    h.renew();
    println("after renew: ", h.locname());
    let hs = [HL { }, HL { }];
    hs[1].loc = L { name: heap("element") };
    println("element: ", hs[1].locname());
}
"#;
    let out = build_run("reassign", src, hale_codegen::BuildOptions { asan: true, ..build_opts::options() });
    let out = clean_stdout("reassign", &out);
    let lines: Vec<&str> = out.lines().filter(|l| !l.chars().all(|c| c.is_ascii_digit())).collect();
    assert_eq!(
        lines,
        [
            "birth FIRST5",
            "dissolve FIRST5",
            "birth OUTSIDE7",
            "after replace: OUTSIDE7",
            "dissolve OUTSIDE7",
            "birth METHOD6",
            "after renew: METHOD6",
            "birth FIRST5",
            "birth FIRST5",
            "dissolve FIRST5",
            "birth ELEMENT7",
            "element: ELEMENT7",
            "dissolve ELEMENT7",
            "dissolve FIRST5",
            "dissolve METHOD6",
        ],
        "{out}"
    );
}
