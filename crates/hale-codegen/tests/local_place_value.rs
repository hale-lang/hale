//! A local bound from a place is yours (spec/types.md § A local bound
//! from a place is yours): writing into it, at any depth, never changes
//! the place it was read from, and writing the place never changes it.
//!
//! An array, a tuple's element, a payload, a `for` element — any
//! compound value read from a field, an element, a local or a parameter
//! without a copy — is the place's own block. Before this rule a write
//! through the local landed in that block: `let mut a = self.r.names;
//! a[0] = x;` changed `self.r.names`, `match self.e { P::A(i) -> i.k = 1
//! }` changed the payload, `for x in self.r.arr { x.k = 1 }` changed the
//! array. Now the local is copied at its first write (one flag per local,
//! so a part is copied once however many writes follow), reads stay
//! zero-copy, and the get-modify-store idiom (`let mut a = self.f; a[i] =
//! y; self.f = a;`) keeps the field's block without a copy. A parameter
//! is still the caller's value by reference (GH #1550): a callee writing
//! through it is seen by the caller.
//!
//! Values are built at run time (`heap`), so a freed block shows as a
//! wrong value or an ASan report, and every binary runs with
//! `LOTUS_NO_CHUNK_POOL=1` (GH #816). Build with `LOTUS_ASAN=1` to run
//! them under the sanitizer.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/sanitize.rs"]
mod sanitize;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_local_place_value_{name}"));
    build_opts::build_source(src, &bin, &sanitize::options()).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{name}: ASan:\n{stderr}");
    stdout
}

/// Build `src` with its IR dumped; the IR.
fn ir(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_local_place_value_ir_{name}"));
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
        .find(|&i| ir[i..].lines().next().is_some_and(|l| l.contains(name) && l.contains('(')))
        .unwrap_or_else(|| panic!("no function {name} in the IR"));
    let rest = &ir[start..];
    &rest[..rest.find("\n}\n").map(|e| e + 2).unwrap_or(rest.len())]
}

/// The copy-on-write blocks a function body holds (`<local>.cow`).
fn cow_blocks(body: &str) -> Vec<&str> {
    body.lines()
        .filter(|l| !l.starts_with(' '))
        .filter_map(|l| l.split(';').next().map(str::trim_end))
        .filter(|label| label.ends_with(':') && label.contains(".cow") && !label.contains(".cow.done"))
        .collect()
}

const TYPES: &str = r#"
type Inner { k: Int = 0; s: String = ""; }
type In { k: Int = 0; s: String = ""; inner: Inner = Inner { }; }
type P = enum { A(In), B };
type R {
    names: [String; 2] = ["", ""];
    arr: [In; 2] = [In { }, In { }];
    ints: [Int; 3] = [0, 0, 0];
    e: P = P::B;
    pair: (In, Int) = (In { }, 0);
}
fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
fn fresh() -> R {
    return R {
        names: [heap("n0"), heap("n1")],
        arr: [In { k: 1, s: heap("a0"), inner: Inner { k: 10, s: heap("i0") } }, In { k: 2, s: heap("a1") }],
        ints: [1, 2, 3],
        e: P::A(In { k: 1, s: heap("p") }),
        pair: (In { k: 1, s: heap("t") }, 1),
    };
}
fn show(label: String, r: R) {
    let pk = match r.e { P::A(i) -> i.k, P::B -> -1, };
    let ps = match r.e { P::A(i) -> i.s, P::B -> "-", };
    println(label, ": n0=", r.names[0], " a0=", r.arr[0].k, "/", r.arr[0].s, " deep=", r.arr[0].inner.k, "/", r.arr[0].inner.s,
        " a1=", r.arr[1].k, " i0=", r.ints[0], " p=", pk, "/", ps, " t=", r.pair.0.k, "/", r.pair.0.s);
}
"#;

/// What every source reads, before and after a local bound from it is
/// written through in every shape.
const UNCHANGED: &str = "n0=N02 a0=1/A02 deep=10/I02 a1=2 i0=1 p=1/P1 t=1/T1";

/// Every shape of a local bound from `src` (an expression of type `R`),
/// written into; each local's own value printed after its write.
fn writes(src: &str) -> String {
    format!(
        r#"
        let mut a = {src}.names;
        a[0] = heap("x");
        let mut b = {src}.arr;
        b[0].k = 7;
        b[0].inner.k = 70;
        b[0].inner.s = heap("deep");
        let mut c = {src}.ints;
        c[0] = 100;
        match {src}.e {{ P::A(i) -> {{ i.k = 5; i.s = heap("y"); println("payload=", i.k, "/", i.s); }}, P::B -> {{ }}, }}
        for x in {src}.arr {{ x.k = x.k + 40; x.inner.k = 9; println("elem=", x.k, "/", x.inner.k); }}
        let (t, n) = {src}.pair;
        t.k = 8;
        t.s = heap("z");
        let mut lit = R {{ arr: {src}.arr }};
        lit.arr[1].k = 99;
        let mut re = R {{ }}.names;
        re = {src}.names;
        re[1] = heap("w");
        println("locals: a0=", a[0], " b0=", b[0].k, "/", b[0].inner.k, "/", b[0].inner.s, " c0=", c[0], " t=", t.k, "/", t.s,
            " lit=", lit.arr[1].k, " re1=", re[1], " n=", n);
        "#
    )
}

const LOCALS: &str = "locals: a0=X1 b0=7/70/DEEP4 c0=100 t=8/Z1 lit=99 re1=W1 n=1";

fn check(name: &str, out: &str) {
    assert!(out.contains(&format!("before: {UNCHANGED}")), "{name}: {out}");
    assert!(out.contains(&format!("after: {UNCHANGED}")), "{name}: the source changed:\n{out}");
    assert!(out.contains(LOCALS), "{name}: the locals lost a write:\n{out}");
    assert!(out.contains("payload=5/Y1"), "{name}: {out}");
    assert!(out.contains("elem=41/9") && out.contains("elem=42/9"), "{name}: {out}");
}

#[test]
fn a_local_bound_from_a_locus_field_is_its_own() {
    let src = format!(
        "{TYPES}
        locus H {{
            params {{ r: R = R {{ }}; }}
            run {{
                self.r = fresh();
                show(\"before\", self.r);
                {}
                show(\"after\", self.r);
            }}
        }}
        fn main() {{ let h = H {{ }}; }}",
        writes("self.r")
    );
    check("locus field", &run("field", &src));
}

#[test]
fn a_local_bound_from_a_record_local_is_its_own() {
    let src = format!(
        "{TYPES}
        fn main() {{
            let r = fresh();
            show(\"before\", r);
            {}
            show(\"after\", r);
        }}",
        writes("r")
    );
    check("record local", &run("local", &src));
}

#[test]
fn a_local_bound_from_a_parameter_is_its_own() {
    // The callee's locals are bound from its parameter: the caller's
    // value, which their writes never reach.
    let src = format!(
        "{TYPES}
        fn work(r: R) {{ {} }}
        fn main() {{
            let r = fresh();
            show(\"before\", r);
            work(r);
            show(\"after\", r);
        }}",
        writes("r")
    );
    check("parameter", &run("param", &src));
}

#[test]
fn a_parameter_written_directly_is_still_the_callers() {
    // GH #1550: an argument is passed by reference; a callee writing
    // through its parameter (not a local bound from it) is seen.
    let src = format!(
        "{TYPES}
        fn poke(r: R) {{ r.names[0] = heap(\"poked\"); r.arr[0].inner.k = 77; }}
        fn poke_arr(a: [In; 2]) {{ a[1].k = 55; }}
        fn main() {{
            let r = fresh();
            poke(r);
            poke_arr(r.arr);
            println(\"caller: \", r.names[0], \" \", r.arr[0].inner.k, \" \", r.arr[1].k);
        }}"
    );
    let out = run("param_ref", &src);
    assert!(out.contains("caller: POKED5 77 55"), "{out}");
}

#[test]
fn a_local_handed_to_a_writing_callee_is_copied() {
    // `a` is bound from the field; the callee writes through its
    // parameter, which is `a` — never the field.
    let src = format!(
        "{TYPES}
        fn poke_arr(a: [In; 2]) {{ a[0].k = 55; a[0].inner.s = heap(\"p\"); }}
        locus H {{
            params {{ r: R = R {{ }}; }}
            run {{
                self.r = fresh();
                let a = self.r.arr;
                poke_arr(a);
                println(\"local=\", a[0].k, \"/\", a[0].inner.s);
                show(\"after\", self.r);
            }}
        }}
        fn main() {{ let h = H {{ }}; }}"
    );
    let out = run("passed", &src);
    assert!(out.contains("local=55/P1"), "{out}");
    assert!(out.contains(&format!("after: {UNCHANGED}")), "{out}");
}

#[test]
fn writing_the_source_never_reaches_a_local_bound_from_it() {
    // The other direction: a record local's array written in place while
    // a local bound from it is live, and a local bound from another local.
    let src = format!(
        "{TYPES}
        fn main() {{
            let r = fresh();
            let a = r.names;
            let deep = r.arr;
            r.names[0] = heap(\"y\");
            r.arr[0].inner.k = 3;
            println(\"held: \", a[0], \" \", deep[0].inner.k);
            let mut x = [heap(\"m\"), heap(\"o\")];
            let y = x;
            x[0] = heap(\"q\");
            let mut z = x;
            z[1] = heap(\"s\");
            println(\"aliases: \", y[0], \" \", x[0], \" \", x[1], \" \", z[1]);
        }}"
    );
    let out = run("reverse", &src);
    assert!(out.contains("held: N02 10"), "{out}");
    assert!(out.contains("aliases: M1 Q1 O1 S1"), "{out}");
}

#[test]
fn a_local_is_copied_once_however_many_writes_follow() {
    // The flag: the first write copies, the rest write the copy — in a
    // loop too, and a rebind to the place clears it again.
    let src = format!(
        "{TYPES}
        locus H {{
            params {{ r: R = R {{ }}; }}
            run {{
                self.r = fresh();
                let mut b = self.r.arr;
                let mut i = 0;
                while i < 2 {{ b[i].k = 20 + i; b[i].inner.s = heap(\"w\"); i = i + 1; }}
                println(\"first: \", b[0].k, \" \", b[1].k, \" \", b[1].inner.s);
                b = self.r.arr;
                b[1].k = 30;
                println(\"rebound: \", b[0].k, \" \", b[1].k);
                show(\"after\", self.r);
            }}
        }}
        fn main() {{ let h = H {{ }}; }}"
    );
    let out = run("once", &src);
    assert!(out.contains("first: 20 21 W1"), "{out}");
    assert!(out.contains("rebound: 1 30"), "{out}");
    assert!(out.contains(&format!("after: {UNCHANGED}")), "{out}");
    let text = ir("once", &src);
    // Two writes before the rebind, one after: three checks of the one
    // flag (a `<local>.cow` block each), never a copy per write.
    let run_body = body(&text, "@H.run(");
    assert_eq!(cow_blocks(run_body).len(), 3, "{run_body}");
}

const STORE_BACK: &str = r#"
type In { k: Int = 0; s: String = ""; }
locus H {
    params { arr: [In; 4] = [In { }, In { }, In { }, In { }]; ints: [Int; 8] = [0, 0, 0, 0, 0, 0, 0, 0]; }
    fn bump(i: Int) {
        let mut a = self.arr;
        a[i % 4].k = a[i % 4].k + 1;
        a[i % 4].s = std::str::upper("v") + to_string(i);
        self.arr = a;
    }
    fn bump_ints(i: Int) {
        let mut a = self.ints;
        a[i % 8] = a[i % 8] + 1;
        self.ints = a;
    }
    fn read(i: Int) -> Int {
        let a = self.arr;
        let n = self.ints;
        return a[i % 4].k + a[(i + 1) % 4].k + n[i % 8];
    }
    fn kept(i: Int) -> Int {
        let mut a = self.arr;
        a[0].k = 1000;
        return a[0].k + self.arr[0].k;
    }
    run {
        let mut i = 0;
        let mut acc = 0;
        while i < 1000 { self.bump(i); self.bump_ints(i); acc = acc + self.read(i); i = i + 1; }
        println("store-back: ", self.arr[1].k, " ", self.arr[3].s, " ", self.ints[7], " ", acc, " ", self.kept(0));
    }
}
fn main() { let h = H { }; }
"#;

#[test]
fn the_get_modify_store_idiom_keeps_the_fields_block() {
    let out = run("store_back", STORE_BACK);
    // 250 bumps of each element; the last write to element 3 was i=999;
    // `kept` copies (no store follows), so the field keeps 250.
    assert!(out.contains("store-back: 250 V999 125 "), "{out}");
    assert!(out.trim_end().ends_with(" 1250"), "{out}");
    let text = ir("store_back", STORE_BACK);
    // A local written then stored back whole into the field it was read
    // from, with nothing reading the field between: no copy on write.
    for f in ["@H.bump(", "@H.bump_ints("] {
        let b = body(&text, f);
        assert!(cow_blocks(b).is_empty(), "{f} copies on write:\n{b}");
    }
    // A read-only binding stays a view: no copy at all.
    let r = body(&text, "@H.read(");
    assert!(cow_blocks(r).is_empty() && !r.contains("clone"), "the read copies:\n{r}");
    // A write with no store after it copies on write.
    assert_eq!(cow_blocks(body(&text, "@H.kept(")).len(), 1);
}

#[test]
fn a_vec_items_walk_still_writes_in_place() {
    // `for x in v.items` visits the vec's cells in place by design
    // (spec/forms.md): unchanged by the local-place rule.
    let src = r#"
        type Pt { x: Int = 0; s: String = ""; }
        @form(vec)
        locus Pts { capacity { heap cells of Pt; } }
        fn main() {
            let v = Pts { };
            v.push(Pt { x: 1, s: std::str::upper("a") });
            v.push(Pt { x: 2, s: std::str::upper("b") });
            for p in v.items { p.x = p.x * 10; }
            let mut s = 0;
            for p in v.items { s = s + p.x; }
            println("sum=", s);
        }
    "#;
    let out = run("vec_items", src);
    assert!(out.contains("sum=30"), "{out}");
}

/// A struct literal holding a field's array (`R { arr: self.source }`)
/// handed to a callee that writes through what it is handed — the
/// literal's part, or the literal whole — leaves the field as it was:
/// the part is copied where the literal is built (review round 3 on
/// #1558).
#[test]
fn a_literal_holding_a_field_handed_to_a_writer_leaves_the_field() {
    let src = r#"
        type In { k: Int = 0; s: String = ""; }
        type R { arr: [In; 2] = [In { }, In { }]; n: Int = 0; }
        fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
        fn poke_arr(xs: [In; 2]) { xs[0].k = 8; xs[0].s = heap("w"); }
        fn poke_rec(r: R) { r.arr[0].k = 9; r.arr[0].s = heap("v"); }
        locus H {
            params { source: [In; 2] = [In { }, In { }]; }
            fn part() -> Int { let lit = R { arr: self.source }; poke_arr(lit.arr); return lit.arr[0].k; }
            fn whole() -> Int { let lit = R { arr: self.source }; poke_rec(lit); return lit.arr[0].k; }
            run {
                self.source = [In { k: 1, s: heap("a") }, In { k: 2 }];
                let p = self.part();
                println("part: local=", p, " field=", self.source[0].k, "/", self.source[0].s);
                let w = self.whole();
                println("whole: local=", w, " field=", self.source[0].k, "/", self.source[0].s);
            }
        }
        fn main() { let h = H { }; }
    "#;
    let out = run("literal_writer", src);
    assert!(out.contains("part: local=8 field=1/A1"), "{out}");
    assert!(out.contains("whole: local=9 field=1/A1"), "{out}");
}

/// The survey and lowering agree, shape by shape: the pass reports a
/// local-place copy in a fn exactly when lowering copies there (a
/// copy-on-write block, or an owned deep copy), and a primitive —
/// a call's `Int`, an alias of one, a distinct `Int`, a `Duration`, a
/// `Decimal`, a `Time`, a `String` — is never one, whatever unknown
/// callee it is handed to (review round 3 on #1558: `let a = one(); let
/// b = a; cb(b);` was a reported copy lowering never made).
#[test]
fn the_survey_and_lowering_agree_on_place_copies() {
    let src = r#"
        type In { k: Int = 0; }
        type R { arr: [In; 2] = [In { }, In { }]; }
        type Count = Int;
        type Hits = distinct Int;
        fn one() -> Int { return 1; }
        fn count() -> Count { return 2; }
        fn hits() -> Hits { return Hits(3); }
        fn dur() -> Duration { return 5ms; }
        fn dec(d: Decimal<2>) -> Decimal<2> { return d; }
        fn word() -> String { return std::str::upper("w"); }
        fn poke_arr(xs: [In; 2]) { xs[0].k = 8; }
        fn twice(x: Int) -> Int { return x * 2; }
        type P { n: Int = 0; pair: (Int, Int) = (1, 2); arr: [In; 2] = [In { }, In { }]; }
        fn arr_of(p: P) -> [In; 2] { return p.arr; }
        locus L { params { v: Int = 3; } fn get() -> Int { return self.v; } }
        locus H {
            params { source: [In; 2] = [In { }, In { }]; }
            fn s_int(cb: fn(Int) -> Int) -> Int { let a = one(); let b = a; return cb(b); }
            fn s_alias(cb: fn(Count) -> Int) -> Int { let a = count(); let b = a; return cb(b); }
            fn s_distinct(cb: fn(Hits) -> Int) -> Int { let a = hits(); let b = a; return cb(b); }
            fn s_duration(cb: fn(Duration) -> Int) -> Int { let a = dur(); let b = a; return cb(b); }
            fn s_decimal(d: Decimal<2>, cb: fn(Decimal<2>) -> Int) -> Int { let a = dec(d); let b = a; return cb(b); }
            fn s_time(cb: fn(Time) -> Int) -> Int { let a = std::time::current(); let b = a; return cb(b); }
            fn s_string(cb: fn(String) -> Int) -> Int { let a = word(); let b = a; return cb(b); }
            fn c_write() -> Int { let mut a = self.source; a[0].k = 4; return a[0].k; }
            fn c_passed() { let a = self.source; poke_arr(a); }
            fn c_literal() { let lit = R { arr: self.source }; poke_arr(lit.arr); }
            fn s_if(cb: fn(Int) -> Int, ok: Bool) -> Int { let a = if ok { 1 } else { 2 }; let b = a; return cb(b); }
            fn s_match(cb: fn(Int) -> Int, n: Int) -> Int { let a = match n { 0 -> 1, _ -> 2, }; let b = a; return cb(b); }
            fn s_block(cb: fn(Int) -> Int, p: P) -> Int { let a = { p.n }; let b = a; return cb(b); }
            fn s_tuple(cb: fn(Int) -> Int) -> Int { let (a, c) = (1, 2); let b = a; return cb(b) + c; }
            fn s_pair(cb: fn(Int) -> Int, p: P) -> Int { let (a, c) = p.pair; let b = a; return cb(b) + c; }
            fn s_method(cb: fn(Int) -> Int, l: L) -> Int { let a = l.get(); let b = a; return cb(b); }
            fn c_assigned() { let mut h = R { }; h.arr = self.source; h.arr[0].k = 5; }
            fn c_returned(p: P) { let mut a = arr_of(p); a[0].k = 5; }
            run { println(self.s_int(twice)); }
        }
        fn main() { let h = H { }; }
    "#;
    // The pass's report, per fn.
    let program = hale_syntax::parse_source(src).expect("parses");
    let views = hale_types::view_bindings::OwnedViews::of(&[&program], &Default::default());
    let reported = |f: &str| -> usize {
        let start = src.find(&format!("fn {f}(")).expect("the fn");
        let end = src[start + 1..].find("\n            fn ").map_or(src.len(), |e| start + 1 + e);
        views
            .cell_copies()
            .iter()
            .filter(|c| hale_types::view_bindings::reasons::is_place(c.reason))
            .filter(|c| (start..end).contains(&(c.span.start.0 as usize)))
            .count()
    };
    let text = ir("agree", src);
    // Lowering's copies, per fn: a copy-on-write block, or an owned deep
    // copy (`emit_return_value_deep_copy`'s fresh blocks and clones).
    let copies = |f: &str| -> usize {
        let b = body(&text, &format!("@H.{f}("));
        let deep = ["fn.ret.arr.alloc", "fn.ret.struct.alloc", "fn.ret.str.clone", "lotus_str_clone_cell_owned"];
        cow_blocks(b).len() + usize::from(deep.iter().any(|d| b.contains(d)))
    };
    for f in [
        "s_int", "s_alias", "s_distinct", "s_duration", "s_decimal", "s_time", "s_string", "s_if", "s_match", "s_block",
        "s_tuple", "s_pair", "s_method",
    ] {
        assert_eq!((reported(f), copies(f)), (0, 0), "{f}: a primitive copies nothing");
    }
    for f in ["c_write", "c_passed", "c_literal", "c_assigned", "c_returned"] {
        let (r, c) = (reported(f), copies(f));
        assert!(r == 1 && c >= 1, "{f}: reported {r}, lowering copies {c}");
    }
}

const REVIEW_TYPES: &str = r#"
type Inner { k: Int = 0; s: String = ""; }
type In { k: Int = 0; s: String = ""; inner: Inner = Inner { }; }
type R { arr: [In; 2] = [In { }, In { }]; }
type Holder { arr: [In; 2] = [In { }, In { }]; }
type Rec { id: Int = 0; inner: Inner = Inner { }; big: [Inner; 2] = [Inner { }, Inner { }]; }
type Wrap { rec: Rec = Rec { }; }
type W { big: [Inner; 2] = [Inner { }, Inner { }]; }
@form(hashmap)
locus M { capacity { pool cells of Rec indexed_by id; } }
fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
fn fresh() -> R {
    return R { arr: [In { k: 1, s: heap("a0"), inner: Inner { k: 10, s: heap("i0") } }, In { k: 2, s: heap("a1") }] };
}
fn freshrec(id: Int) -> Rec {
    return Rec { id: id, inner: Inner { k: 1, s: heap("in") }, big: [Inner { k: 2, s: heap("b0") }, Inner { k: 3 }] };
}
fn show(label: String, r: R) { println(label, ": a0=", r.arr[0].k, "/", r.arr[0].inner.k, " a1=", r.arr[1].k); }
fn showc(label: String, m: M) { let c = m.get(1) or Rec { }; println(label, ": k=", c.inner.k, " s=", c.inner.s, " b0=", c.big[0].k); }
fn poke_arr(a: [In; 2]) { a[0].k = 7; a[0].inner.k = 70; }
"#;

/// A place's or a cell's value assigned into a part of a local (review of
/// #1558, finding 1): the local holds that block now, as a literal field
/// init would, and a write through it — or handing the part to a writer —
/// copies the part first.
#[test]
fn a_place_assigned_into_a_part_of_a_local_is_copied_at_its_write() {
    let src = format!(
        "{REVIEW_TYPES}
        fn work(r: R) {{ let mut h = Holder {{ }}; h.arr = r.arr; h.arr[1].k = 99; }}
        fn poke_w(r: Rec) {{ let mut h = W {{ }}; h.big = r.big; h.big[0].k = 7; }}
        fn main() {{
            let r = fresh();
            let mut h = Holder {{ }};
            h.arr = r.arr;
            h.arr[0].k = 7;
            h.arr[0].inner.k = 70;
            println(\"local h=\", h.arr[0].k, \"/\", h.arr[0].inner.k);
            show(\"field\", r);
            let mut g = Holder {{ }};
            g.arr = r.arr;
            poke_arr(g.arr);
            show(\"handed\", r);
            work(r);
            show(\"callee\", r);
            let m = M {{ }};
            m.set(freshrec(1));
            let mut w = Wrap {{ }};
            w.rec = m.get(1) or Rec {{ }};
            w.rec.inner.k = 7;
            w.rec.big[0].k = 8;
            println(\"wrap=\", w.rec.inner.k, \"/\", w.rec.big[0].k);
            showc(\"cell field\", m);
            let mut xs = [Rec {{ }}, Rec {{ }}];
            xs[0] = m.get(1) or Rec {{ }};
            xs[0].inner.k = 7;
            xs[0].big[0].k = 8;
            showc(\"cell element\", m);
            let c = m.get(1) or Rec {{ }};
            poke_w(c);
            showc(\"cell callee\", m);
        }}"
    );
    let out = run("part_assign", &src);
    assert!(out.contains("local h=7/70"), "{out}");
    for label in ["field", "handed", "callee"] {
        assert!(out.contains(&format!("{label}: a0=1/10 a1=2")), "{label}:\n{out}");
    }
    assert!(out.contains("wrap=7/8"), "{out}");
    for label in ["cell field", "cell element", "cell callee"] {
        assert!(out.contains(&format!("{label}: k=1 s=IN2 b0=2")), "{label}:\n{out}");
    }
}

/// A call's result can be a place's value (review of #1558, finding 2): a
/// fn returning a part of its parameter hands back the caller's block, and
/// a locus method returning a cell's read hands back the cell's nested
/// blocks. A local bound from such a result is a local bound from a place,
/// copied at its first write.
#[test]
fn a_call_returning_a_place_hands_back_a_place_bound_local() {
    let src = format!(
        "{REVIEW_TYPES}
        fn get_in(r: R) -> In {{ return r.arr[0]; }}
        fn get_arr(r: R) -> [In; 2] {{ return r.arr; }}
        fn part(r: Rec) -> [Inner; 2] {{ return r.big; }}
        fn poke_part(r: Rec) {{ let mut a = part(r); a[0].k = 7; }}
        locus H {{
            params {{ m: M = M {{ }}; }}
            fn read() -> Rec {{ return self.m.get(1) or Rec {{ }}; }}
            run {{
                self.m.set(freshrec(1));
                let mut c = self.read();
                c.inner.s = heap(\"zz\");
                c.big[0].k = 9;
                println(\"method c=\", c.inner.s, \"/\", c.big[0].k);
                showc(\"method\", self.m);
            }}
        }}
        fn main() {{
            let r = fresh();
            let mut x = get_in(r);
            x.k = 7;
            x.inner.k = 70;
            println(\"x=\", x.k, \"/\", x.inner.k);
            show(\"get_in\", r);
            let mut a = get_arr(r);
            a[0].k = 7;
            show(\"get_arr\", r);
            let m = M {{ }};
            m.set(freshrec(1));
            let c = m.get(1) or Rec {{ }};
            let mut b = part(c);
            b[0].k = 7;
            showc(\"part bound\", m);
            let mut d = part(m.get(1) or Rec {{ }});
            d[0].k = 7;
            showc(\"part inline\", m);
            let e = m.get(1) or Rec {{ }};
            poke_part(e);
            showc(\"part writer\", m);
            let h = H {{ }};
        }}"
    );
    let out = run("call_result", &src);
    assert!(out.contains("x=7/70"), "{out}");
    assert!(out.contains("get_in: a0=1/10 a1=2") && out.contains("get_arr: a0=1/10 a1=2"), "{out}");
    for label in ["part bound", "part inline", "part writer"] {
        assert!(out.contains(&format!("{label}: k=1 s=IN2 b0=2")), "{label}:\n{out}");
    }
    assert!(out.contains("method c=ZZ2/9"), "{out}");
    assert!(out.contains("method: k=1 s=IN2 b0=2"), "{out}");
}
