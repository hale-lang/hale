//! A value read from a form's cell is the reader's (spec/forms.md §
//! Reading a cell): writing into it never changes the cell — only `set`,
//! `remove` and the form's other ops do.
//!
//! A map that reads in place (no `sync =`, `sync = none`) and an LRU
//! cache hand back the cell's bytes copied into the reader's frame, its
//! nested blocks still the cell's; a write into one of those parts went
//! straight into the cell (`let mut c = m.get(1) or R { }; c.inner.k =
//! 7;` changed the cell with no `set`). The compiler now copies the part
//! at its first write, unless the write is part of a get-modify-set it
//! can prove no one sees (`hale_types::view_bindings`), which stays
//! allocation-free. The synced maps already read a deep copy; the
//! repro holds on every discipline here.
//!
//! The values are built at run time (`heap`), so a freed block shows
//! as a wrong value or an ASan report, and every binary runs with
//! `LOTUS_NO_CHUNK_POOL=1`, so a use after a free is not hidden by a
//! chunk handed back out with its bytes intact (GH #816). Build with
//! `LOTUS_ASAN=1` to run them under the sanitizer.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/sanitize.rs"]
mod sanitize;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_form_read_value_{name}"));
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
    let bin = harness::unique_bin(&format!("hale_form_read_value_ir_{name}"));
    let ll: PathBuf = bin.with_extension("ll");
    let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let text = std::fs::read_to_string(&ll).unwrap_or_else(|e| panic!("{name}: read the IR: {e}"));
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    text
}

/// The body of the function `@name`.
fn body<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("@{name}(");
    let start = ir
        .match_indices("define ")
        .map(|(i, _)| i)
        .find(|&i| ir[i..].lines().next().is_some_and(|l| l.contains(&head)))
        .unwrap_or_else(|| panic!("no function {name} in the IR"));
    let rest = &ir[start..];
    &rest[..rest.find("\n}\n").map(|e| e + 2).unwrap_or(rest.len())]
}

/// The copy-on-write blocks a function body holds (`<local>.<part>.cow`).
fn cow_blocks(body: &str) -> Vec<&str> {
    body.lines()
        .filter(|l| !l.starts_with(' '))
        .filter_map(|l| l.split(';').next().map(str::trim_end))
        .filter(|label| label.ends_with(':') && label.contains(".cow") && !label.contains(".cow.done"))
        .collect()
}

/// The functions a body calls, sorted, debug-info intrinsics aside.
fn callees(body: &str) -> Vec<&str> {
    let mut out: Vec<&str> = body
        .match_indices(" @")
        .filter(|(i, _)| body[..*i].ends_with("call ptr") || body[..*i].ends_with("call void") || body[..*i].ends_with("call i32") || body[..*i].ends_with("call i1") || body[..*i].ends_with("call i64"))
        .map(|(i, _)| {
            let rest = &body[i + 2..];
            &rest[..rest.find('(').unwrap_or(rest.len())]
        })
        .filter(|n| !n.starts_with("llvm.dbg"))
        .collect();
    out.sort_unstable();
    out
}

const TYPES: &str = r#"
type Tag = enum { Named(String), Num(Int) };
type Inner { s: String = ""; k: Int = 0; tag: Tag = Tag::Num(0); b: Bytes = b""; pair: (Int, String) = (0, ""); }
type Rec { id: Int = 0; body: String = ""; inner: Inner = Inner { }; names: [String; 2] = ["", ""]; }
fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
fn tag(t: Tag) -> String { return match t { Tag::Named(s) -> s, Tag::Num(n) -> to_string(n), }; }
fn show(label: String, r: Rec) {
    println(label, ": body=", r.body, " s=", r.inner.s, " k=", r.inner.k, " tag=", tag(r.inner.tag),
        " b=", len(r.inner.b), " pair=", r.inner.pair.1, " n1=", r.names[1]);
}
fn fresh(id: Int) -> Rec {
    return Rec { id: id, body: heap("body"), names: [heap("n0"), heap("n1")],
        inner: Inner { s: heap("in"), k: 1, tag: Tag::Named(heap("tag")), b: std::bytes::from_string(heap("bytes")), pair: (1, heap("pair")) } };
}
"#;

/// What the cell reads, before and after a reader writes into every
/// nested part of its value (and the value's own top level).
const CELL_BEFORE: &str = "cell: body=BODY4 s=IN2 k=1 tag=TAG3 b=6 pair=PAIR4 n1=N12";
const COPY_AFTER: &str = "copy: body=BODY4-top s=IN2-changed k=7 tag=9 b=2 pair=Q1 n1=CHANGED7";

fn writes(local: &str) -> String {
    format!(
        "{local}.body = {local}.body + \"-top\";
        {local}.inner.k = 7;
        {local}.inner.s = {local}.inner.s + \"-changed\";
        {local}.inner.tag = Tag::Num(9);
        {local}.inner.b = std::bytes::from_string(heap(\"z\"));
        {local}.inner.pair = (2, heap(\"q\"));
        {local}.names[1] = heap(\"changed\");"
    )
}

fn map_program(form: &str) -> String {
    format!(
        "{TYPES}
{form}
locus RecMap {{ capacity {{ pool items of Rec indexed_by id; }} }}
fn main() {{
    let m = RecMap {{ }};
    m.set(fresh(1));
    let mut c = m.get(1) or Rec {{ }};
    {get_writes}
    show(\"copy\", c);
    show(\"cell\", m.get(1) or Rec {{ }});
    let mut e = m.entry_at(0) or Rec {{ }};
    {entry_writes}
    show(\"copy\", e);
    show(\"cell\", m.get(1) or Rec {{ }});
}}
",
        get_writes = writes("c"),
        entry_writes = writes("e"),
    )
}

#[test]
fn a_write_into_a_maps_read_never_changes_the_cell() {
    for (tag, form) in [
        ("plain", "@form(hashmap)"),
        ("none", "@form(hashmap, sync = none)"),
        ("serialized", "@form(hashmap, sync = serialized)"),
        ("striped", "@form(hashmap, sync = striped)"),
        ("lockfree", "@form(hashmap, sync = lockfree)"),
    ] {
        let out = run(&format!("map_{tag}"), &map_program(form));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, [COPY_AFTER, CELL_BEFORE, COPY_AFTER, CELL_BEFORE], "{tag}:\n{out}");
    }
}

#[test]
fn a_write_into_an_lru_caches_read_never_changes_the_cell() {
    let src = format!(
        "{TYPES}
@form(lru_cache, cap = 8)
locus Cache {{ capacity {{ pool items of Rec indexed_by id; }} }}
fn main() {{
    let m = Cache {{ }};
    m.put(fresh(1));
    let mut c = m.get(1) or Rec {{ }};
    {writes}
    show(\"copy\", c);
    show(\"cell\", m.get(1) or Rec {{ }});
}}
",
        writes = writes("c")
    );
    let out = run("lru", &src);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, [COPY_AFTER, CELL_BEFORE], "{out}");
}

/// The repro the rule was settled on, in a method of a locus holding the
/// map: the nested record written, the cell read again.
#[test]
fn the_repro_holds_in_a_method_on_a_held_map() {
    let src = r#"
        type Inner { s: String = ""; k: Int = 0; }
        type Rec { id: Int = 0; body: String = ""; inner: Inner = Inner { }; }
        @form(hashmap)
        locus RecMap { capacity { pool items of Rec indexed_by id; } }
        fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
        locus Holder {
            params { m: RecMap = RecMap { }; }
            fn go() {
                self.m.set(Rec { id: 1, body: heap("b"), inner: Inner { s: heap("in"), k: 1 } });
                let mut c = self.m.get(1) or Rec { };
                c.inner.k = 7;
                c.inner.s = c.inner.s + "-changed";
                let again = self.m.get(1) or Rec { };
                println("cell: inner.s=", again.inner.s, " inner.k=", again.inner.k);
            }
        }
        fn main() {
            let h = Holder { };
            h.go();
        }
    "#;
    assert_eq!(run("held", src).trim(), "cell: inner.s=IN2 inner.k=1");
}

/// The get-modify-set idiom keeps the read shared: the writes land in
/// the cell's blocks, which the set keeps, and nothing is copied.
#[test]
fn the_get_modify_set_idiom_copies_nothing() {
    let src = r#"
        type Level { p: Int = 0; q: Int = 0; }
        type Big { id: Int = 0; name: String = ""; big: [Level; 8] = [Level { }; 8]; inner: Level = Level { }; }
        @form(hashmap)
        locus M { capacity { pool cells of Big indexed_by id; } }
        fn idiom(m: M, k: Int, j: Int, x: Int) {
            let mut e = m.get(k) or Big { id: k };
            e.big[j] = Level { p: x, q: x };
            e.inner.p = x;
            m.set(e);
        }
        fn untouched(m: M, k: Int, j: Int, x: Int) {
            let mut e = m.get(k) or Big { id: k };
            m.set(e);
        }
        fn keyed(m: M, k: Int, x: Int) {
            let mut e = m.get(k) or Big { id: k };
            e.inner.q = x;
            m.set(Big { id: k, name: e.name, big: e.big, inner: e.inner });
        }
        fn main() {
            let m = M { };
            m.set(Big { id: 1, name: "one" });
            idiom(m, 1, 3, 7);
            keyed(m, 1, 9);
            let a = m.get(1) or Big { };
            println(a.big[3].p, " ", a.inner.p, " ", a.inner.q);
        }
    "#;
    assert_eq!(run("idiom", src).trim(), "7 7 9");
    let ir = ir("idiom", src);
    for f in ["idiom", "keyed"] {
        let b = body(&ir, f);
        assert!(cow_blocks(b).is_empty(), "{f} copies on write:\n{b}");
    }
    // The writes add no call to the get and the set: what `idiom` calls
    // beyond `untouched` is the literal its element write builds.
    let (with, without) = (callees(body(&ir, "idiom")), callees(body(&ir, "untouched")));
    let mut extra = with.clone();
    for c in &without {
        if let Some(i) = extra.iter().position(|x| x == c) {
            extra.remove(i);
        }
    }
    assert!(extra.iter().all(|c| *c == "lotus_arena_alloc_reusable" || *c == "lotus_arena_alloc"), "idiom calls {extra:?} beyond the get and the set");
    assert!(extra.len() <= 1, "idiom allocates {extra:?} beyond its element literal");
}

/// A write the cell would show — the cell is read again before the set,
/// or nothing sets the value back — copies the part at its first write,
/// once: both values are right.
#[test]
fn a_write_the_cell_would_show_copies_the_part_once() {
    let src = r#"
        type Level { p: Int = 0; q: Int = 0; }
        type Big { id: Int = 0; big: [Level; 4] = [Level { }; 4]; inner: Level = Level { }; }
        @form(hashmap)
        locus M { capacity { pool cells of Big indexed_by id; } }
        fn peek(mm: M, k: Int) -> Int { let c = mm.get(k) or Big { }; return c.inner.p; }
        fn read_between(m: M, k: Int) -> Int {
            let mut e = m.get(k) or Big { id: k };
            e.inner.p = 70;
            e.inner.q = 71;
            let seen = peek(m, k);
            m.set(e);
            return seen;
        }
        fn no_set(m: M, k: Int) -> Int {
            let mut e = m.get(k) or Big { id: k };
            let mut i = 0;
            while i < 4 { e.big[i] = Level { p: 80 + i }; i = i + 1; }
            return e.big[3].p;
        }
        fn main() {
            let m = M { };
            m.set(Big { id: 1, inner: Level { p: 1, q: 2 } });
            let seen = read_between(m, 1);
            let a = m.get(1) or Big { };
            println("read_between: seen=", seen, " cell=", a.inner.p, "/", a.inner.q);
            let got = no_set(m, 1);
            let b = m.get(1) or Big { };
            println("no_set: got=", got, " cell=", b.big[3].p);
        }
    "#;
    assert_eq!(run("cow", src), "read_between: seen=1 cell=70/71\nno_set: got=83 cell=0\n");
    let ir = ir("cow", src);
    // One copy-on-write block per write, each behind the part's flag:
    // the second write into `inner` finds it owned.
    assert_eq!(cow_blocks(body(&ir, "read_between")).len(), 2, "{}", body(&ir, "read_between"));
    assert_eq!(cow_blocks(body(&ir, "no_set")).len(), 1, "{}", body(&ir, "no_set"));
}

/// A value read from a cell handed to a callee that writes through its
/// parameter is the caller's own (the caller sees the write; the cell
/// does not), and so is an entry of a map's walk the body writes into.
#[test]
fn a_read_handed_to_a_writer_or_walked_and_written_is_owned() {
    let src = r#"
        type Inner { s: String = ""; k: Int = 0; }
        type Rec { id: Int = 0; inner: Inner = Inner { }; }
        @form(hashmap)
        locus RecMap { capacity { pool items of Rec indexed_by id; } }
        fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
        fn poke(r: Rec) { r.inner.k = 99; r.inner.s = heap("poked"); }
        fn main() {
            let m = RecMap { };
            m.set(Rec { id: 1, inner: Inner { s: heap("in"), k: 1 } });
            let c = m.get(1) or Rec { };
            poke(c);
            println("caller=", c.inner.k, " cell=", (m.get(1) or Rec { }).inner.k);
            poke(m.get(1) or Rec { });
            println("direct cell=", (m.get(1) or Rec { }).inner.k);
            for e in m.entries { e.inner.k = 5; e.id = 9; }
            let a = m.get(1) or Rec { };
            println("walked cell=", a.inner.k, " ", a.inner.s, " has9=", m.has(9));
        }
    "#;
    assert_eq!(run("handed", src), "caller=99 cell=1\ndirect cell=1\nwalked cell=1 IN2 has9=false\n");
}

/// A callee writes through its parameter by more than an assignment
/// rooted at it: by a local bound from a part of it, by a `for` element
/// of it, and by handing such a local to another writer. Each makes the
/// caller's read its own, so the cell keeps its value.
#[test]
fn a_callee_writing_through_an_alias_of_its_parameter_leaves_the_cell() {
    let src = r#"
        type In { s: String = ""; k: Int = 0; }
        type Rec { id: Int = 0; big: [In; 2] = [In { }, In { }]; }
        @form(hashmap)
        locus RecMap { capacity { pool items of Rec indexed_by id; } }
        fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
        fn poke_alias(r: Rec) { let mut a = r.big; a[0].k = 7; a[0].s = heap("alias"); }
        fn poke_for(r: Rec) { for x in r.big { x.k = 8; } }
        fn poke_arr(xs: [In; 2]) { xs[1].k = 9; }
        fn poke_hand(r: Rec) { let a = r.big; poke_arr(a); }
        fn cell(m: RecMap) -> String {
            let c = m.get(1) or Rec { };
            return to_string(c.big[0].k) + "/" + c.big[0].s + "/" + to_string(c.big[1].k);
        }
        fn main() {
            let m = RecMap { };
            m.set(Rec { id: 1, big: [In { s: heap("a"), k: 1 }, In { s: heap("b"), k: 2 }] });
            let c = m.get(1) or Rec { };
            poke_alias(c);
            println("alias cell=", cell(m));
            poke_alias(m.get(1) or Rec { });
            println("alias direct cell=", cell(m));
            let d = m.get(1) or Rec { };
            poke_for(d);
            println("for cell=", cell(m));
            poke_for(m.get(1) or Rec { });
            println("for direct cell=", cell(m));
            let e = m.get(1) or Rec { };
            poke_hand(e);
            println("hand cell=", cell(m));
            poke_hand(m.get(1) or Rec { });
            println("hand direct cell=", cell(m));
        }
    "#;
    let want: String = ["alias", "alias direct", "for", "for direct", "hand", "hand direct"]
        .iter()
        .map(|w| format!("{w} cell=1/A1/2\n"))
        .collect();
    assert_eq!(run("callee_alias", src), want);
}

/// The owned-view pass and lowering read an array field's layout by one
/// rule: a write into an inline scalar array (however its element is
/// spelled) is the local's own, so the pass reports no copy and lowering
/// emits none; a write into an array of records copies the array, and
/// both say so.
#[test]
fn the_survey_and_lowering_agree_on_inline_arrays() {
    for (tag, elem, init, value, inline) in [
        ("int", "Int", "[0, 0]", "4", true),
        ("alias", "Count", "[0, 0]", "4", true),
        ("distinct", "Hits", "[Hits(0), Hits(0)]", "Hits(4)", true),
        ("float", "Float", "[0.0, 0.0]", "4.0", true),
        ("record", "Lvl", "[Lvl { }, Lvl { }]", "Lvl { n: 4 }", false),
    ] {
        let src = format!(
            r#"
type Count = Int;
type Hits = distinct Int;
type Lvl {{ n: Int = 0; }}
type Rec {{ id: Int = 0; xs: [{elem}; 2] = {init}; }}
@form(hashmap)
locus RecMap {{ capacity {{ pool items of Rec indexed_by id; }} }}
fn write(m: RecMap) -> Int {{
    let mut e = m.get(1) or Rec {{ }};
    e.xs[1] = {value};
    return e.id;
}}
fn main() {{
    let m = RecMap {{ }};
    m.set(Rec {{ id: 1 }});
    println(write(m));
}}
"#
        );
        let program = hale_syntax::parse_source(&src).expect("parses");
        let views = hale_types::view_bindings::OwnedViews::of(&[&program], &Default::default());
        let reported = views.cell_copies().iter().any(|c| src[c.span.start.0 as usize..c.span.end.0 as usize].starts_with("e.xs[1]"));
        let ir = ir(&format!("inline_{tag}"), &src);
        let copied = !cow_blocks(body(&ir, "write")).is_empty();
        assert_eq!(reported, copied, "{tag}: the survey says {reported}, lowering copies {copied}");
        assert_eq!(copied, !inline, "{tag}");
    }
}

/// Four more ways a callee's local may alias its parameter (review round
/// 2 on #1558): a block's result, either branch of an `if`, a `match`
/// payload, and a loop-carried rebind. Each is a local bound from a place
/// (a part of the parameter), so the local-place rule copies it at its
/// first write (spec/types.md § A local bound from a place is yours) —
/// the write never reaches the caller's read, so the cell keeps its
/// value; the writes-through-parameter flag stays as a second net.
#[test]
fn a_callee_writing_a_block_if_match_or_loop_alias_leaves_the_cell() {
    let src = r#"
        type Inner { k: Int = 0; s: String = ""; }
        type Payload = enum { Some([Inner; 2]), None };
        type Rec { id: Int = 0; big: [Inner; 2] = [Inner { }; 2]; p: Payload = Payload::None; }
        @form(hashmap)
        locus M { capacity { pool cells of Rec indexed_by id; } }
        fn heap(s: String) -> String { return std::str::upper(s) + to_string(len(s)); }
        fn poke_block(r: Rec) { let mut a = { let xs = r.big; xs }; a[0].k = 7; a[0].s = heap("w"); }
        fn poke_if(r: Rec, f: Bool) {
            let mut a = if f { let xs = r.big; xs } else { let ys = r.big; ys };
            a[0].k = 7;
            a[0].s = heap("w");
        }
        fn poke_match(r: Rec) {
            let mut a = match r.p { Payload::Some(xs) -> xs, Payload::None -> [Inner { }; 2], };
            a[0].k = 7;
            a[0].s = heap("w");
        }
        fn poke_loop(r: Rec) {
            let mut a = [Inner { }; 2];
            let mut i = 0;
            while i < 2 { a[0].k = 7; a[0].s = heap("w"); a = r.big; i = i + 1; }
        }
        fn cell(m: M) -> String {
            let c = m.get(1) or Rec { };
            let pk = match c.p { Payload::Some(xs) -> to_string(xs[0].k) + "/" + xs[0].s, Payload::None -> "-", };
            return to_string(c.big[0].k) + "/" + c.big[0].s + " p=" + pk;
        }
        fn main() {
            let m = M { };
            m.set(Rec { id: 1, big: [Inner { k: 1, s: heap("a") }, Inner { k: 2 }], p: Payload::Some([Inner { k: 3, s: heap("p") }, Inner { }]) });
            let c = m.get(1) or Rec { };
            poke_block(c);
            println("block cell=", cell(m));
            poke_block(m.get(1) or Rec { });
            println("block direct cell=", cell(m));
            let d = m.get(1) or Rec { };
            poke_if(d, true);
            poke_if(d, false);
            println("if cell=", cell(m));
            poke_if(m.get(1) or Rec { }, true);
            println("if direct cell=", cell(m));
            let e = m.get(1) or Rec { };
            poke_match(e);
            println("match cell=", cell(m));
            poke_match(m.get(1) or Rec { });
            println("match direct cell=", cell(m));
            let f = m.get(1) or Rec { };
            poke_loop(f);
            println("loop cell=", cell(m));
            poke_loop(m.get(1) or Rec { });
            println("loop direct cell=", cell(m));
        }
    "#;
    let want: String = ["block", "block direct", "if", "if direct", "match", "match direct", "loop", "loop direct"]
        .iter()
        .map(|w| format!("{w} cell=1/A1 p=3/P1\n"))
        .collect();
    assert_eq!(run("callee_alias_round2", src), want);
}
