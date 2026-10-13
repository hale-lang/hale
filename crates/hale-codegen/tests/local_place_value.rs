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
