//! GH #2105: a whole store into a local's `bounded` field frees the
//! elements the field's previous such copy made, when no other name in
//! scope reaches them (`hale_types::field_retire`).
//!
//! `q.tags = r.tags` copies each element whole (GH #2099), and nothing
//! freed the copy it replaced: repeated in a loop over a local declared
//! before it, the frame's arena grew by a copy per store (117 MB over
//! 200,000 stores of four nested-record tags, in `fn main`, a free fn and
//! a method alike). A store the analysis admits copies into an arena of
//! the field's own, made fresh at each such store; what it frees is only
//! what those stores made, so a block that reached the field another way
//! (a push of a live String) is never freed, and a store with a name in
//! scope that may hold a block of an earlier copy (a binding read from the
//! field, a place one was stored into, the local itself) keeps the frame's
//! arena as before. Those controls are built under AddressSanitizer with
//! the chunk pool off, so a block freed while a name still reaches it is a
//! reported use-after-free.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The issue's shape (a local declared before the loop), a nested field
/// path, the store in a locus method and in `fn main`'s own loop.
const SHAPES: &str = r#"
type Meta { note: String = ""; }
type Tag { name: String = ""; meta: Meta = Meta { }; }
type Row { id: String = ""; tags: bounded[Tag; 4]; }
type Outer { label: String = ""; row: Row = Row { }; }

fn source(k: Int) -> Row {
    let mut r = Row { id: f"r{k}" };
    let mut j = 0;
    while j < 4 {
        push(r.tags, Tag { name: f"t{k}-{j}", meta: Meta { note: f"n{k}-{j}" } }) or discard;
        j = j + 1;
    }
    return r;
}

// the issue's shape: a local declared before the loop
fn flat_outer(n: Int) -> Int {
    let r = source(1);
    let mut q = Row { id: "q" };
    let mut total = 0;
    let mut i = 0;
    while i < n {
        q.tags = r.tags;
        total = total + count(q.tags);
        i = i + 1;
    }
    return total + len((at(q.tags, 3) or Tag { }).meta.note);
}

// nested field path
fn flat_nested(n: Int) -> Int {
    let r = source(2);
    let mut o = Outer { label: "o" };
    let mut total = 0;
    let mut i = 0;
    while i < n {
        o.row.tags = r.tags;
        total = total + count(o.row.tags);
        i = i + 1;
    }
    return total + len((at(o.row.tags, 0) or Tag { }).name);
}

locus Holder {
    params { total: Int = 0; }
    fn churn(n: Int) {
        let r = source(3);
        let mut q = Row { };
        let mut i = 0;
        while i < n {
            q.tags = r.tags;
            self.total = self.total + count(q.tags);
            i = i + 1;
        }
    }
}

fn main() {
    let n = 50000;
    println("outer ", flat_outer(n));
    println("nested ", flat_nested(n));
    let h = Holder { };
    h.churn(n);
    println("method ", h.total);
    // fn main's own loop, q declared before it
    let r = source(4);
    let mut q = Row { };
    let mut i = 0;
    let mut t = 0;
    while i < n {
        q.tags = r.tags;
        t = t + count(q.tags);
        i = i + 1;
    }
    println("main ", t, " ", (at(q.tags, 2) or Tag { }).meta.note);
}
"#;

/// Elements a name outside the field reaches, each read after the stores:
/// a binding read from the field before the loop (a), an element pushed
/// from a live String (b, which retires: the String is not the field's),
/// an `at` alias in the loop used after the next store (c), an element's
/// String stored into another record (d) and into the local's own other
/// field (e), a `for` over the list storing into it (f), the local pushed
/// whole (g) and moved (h) into an outer list, the local handed whole to a
/// callee (i), and a local declared in a body with an iteration arena (j).
const CONTROLS: &str = r#"
type Meta { note: String = ""; }
type Tag { name: String = ""; meta: Meta = Meta { }; }
type Row { id: String = ""; label: String = ""; tags: bounded[Tag; 4]; }
type Rows { items: bounded[Row; 8]; }

fn source(k: Int) -> Row {
    let mut r = Row { id: f"r{k}" };
    let mut j = 0;
    while j < 3 {
        push(r.tags, Tag { name: f"t{k}-{j}", meta: Meta { note: f"n{k}-{j}" } }) or discard;
        j = j + 1;
    }
    return r;
}

// a: a binding read from the field before the loop, read after it
fn alias_before() -> String {
    let r = source(1);
    let mut q = Row { };
    q.tags = r.tags;
    let t = at(q.tags, 0) or Tag { };
    let mut i = 0;
    while i < 50 { q.tags = r.tags; i = i + 1; }
    return t.name + "/" + t.meta.note;
}

// b: an element pushed from a live String, then whole stores: retires, s survives
fn pushed_live() -> String {
    let r = source(2);
    let s = f"live-{2}";
    let mut q = Row { };
    push(q.tags, Tag { name: s }) or discard;
    let mut i = 0;
    while i < 50 { q.tags = r.tags; i = i + 1; }
    return s + " " + (at(q.tags, 1) or Tag { }).name;
}

// c: aliased through `at` inside the loop, used after the next store
fn alias_in_loop() -> String {
    let r = source(3);
    let mut q = Row { };
    q.tags = r.tags;
    let mut last = "";
    let mut i = 0;
    while i < 50 {
        let t = at(q.tags, 0) or Tag { };
        q.tags = r.tags;
        last = t.meta.note;
        i = i + 1;
    }
    return last;
}

// d: a record's String taken from an element
fn string_into_record() -> String {
    let r = source(4);
    let mut q = Row { };
    let mut other = Row { };
    let mut i = 0;
    while i < 50 {
        q.tags = r.tags;
        other.label = (at(q.tags, 2) or Tag { }).name;
        i = i + 1;
    }
    return other.label;
}

// e: the field's own other field takes an element's String
fn into_own_label() -> String {
    let r = source(5);
    let mut q = Row { };
    let mut i = 0;
    while i < 50 {
        q.tags = r.tags;
        q.label = (at(q.tags, 0) or Tag { }).meta.note;
        i = i + 1;
    }
    return q.label;
}

// f: a for over the list, storing into it inside
fn store_in_for() -> String {
    let r = source(6);
    let mut q = Row { };
    q.tags = r.tags;
    let mut seen = "";
    for t in q.tags {
        q.tags = r.tags;
        seen = seen + t.name;
    }
    return seen;
}

// g: the local pushed into an outer list each iteration (a copy while it is live)
fn pushed_whole() -> String {
    let r = source(7);
    let mut rows = Rows { };
    let mut q = Row { };
    let mut i = 0;
    while i < 8 {
        q.tags = r.tags;
        push(rows.items, q) or discard;
        i = i + 1;
    }
    let first = at(rows.items, 0) or Row { };
    return (at(first.tags, 1) or Tag { }).meta.note;
}

// h: a local declared in the body and moved into an outer list
fn moved_out() -> String {
    let r = source(8);
    let mut rows = Rows { };
    let mut i = 0;
    while i < 8 {
        let mut q = Row { };
        q.tags = r.tags;
        q.tags = r.tags;
        push(rows.items, q) or discard;
        i = i + 1;
    }
    let first = at(rows.items, 0) or Row { };
    return (at(first.tags, 2) or Tag { }).name;
}

fn label_from(q: Row) -> String { return (at(q.tags, 0) or Tag { }).name; }

// i: the local handed whole to a callee
fn handed_whole() -> String {
    let r = source(9);
    let mut q = Row { };
    let mut keep = "";
    let mut i = 0;
    while i < 50 {
        q.tags = r.tags;
        keep = label_from(q);
        i = i + 1;
    }
    return keep;
}

// j: the local declared in a loop body on an iteration arena
fn body_local(n: Int) -> Int {
    let r = source(10);
    let mut total = 0;
    let mut i = 0;
    while i < n {
        let mut q = Row { };
        q.tags = r.tags;
        q.tags = r.tags;
        total = total + len((at(q.tags, 0) or Tag { }).meta.note);
        i = i + 1;
    }
    return total;
}

fn main() {
    println("a ", alias_before());
    println("b ", pushed_live());
    println("c ", alias_in_loop());
    println("d ", string_into_record());
    println("e ", into_own_label());
    println("f ", store_in_for());
    println("g ", pushed_whole());
    println("h ", moved_out());
    println("i ", handed_whole());
    println("j ", body_local(100));
}
"#;

const CONTROLS_OUT: &str = "a t1-0/n1-0\nb live-2 t2-1\nc n3-0\nd t4-2\ne n5-0\nf t6-0t6-1t6-2\ng n7-1\nh t8-2\ni t9-0\nj 500\n";

fn run(name: &str, src: &str, asan: bool, env: &[(&str, &str)]) -> (String, String) {
    let bin = harness::unique_bin(&format!("local_field_retire_{name}_{asan}"));
    let opts = hale_codegen::BuildOptions { asan, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).expect("build");
    let mut cmd = Command::new(&bin);
    cmd.env("ASAN_OPTIONS", "detect_leaks=0");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{name} asan={asan}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{name}: ASan report:\n{stderr}");
    (stdout, stderr)
}

/// 50,000 stores in each shape hold one copy at a time: the arenas' peak
/// stays under 1 MiB, where every copy kept was about 30 MiB.
#[test]
fn a_whole_store_into_a_locals_field_in_a_loop_stays_flat() {
    let (out, err) = run("shapes", SHAPES, false, &[("LOTUS_ARENA_RESIDENCY", "1")]);
    assert_eq!(out, "outer 200004\nnested 200004\nmethod 200000\nmain 200000 n4-2\n");
    let peak: u64 = err
        .lines()
        .find_map(|l| l.strip_prefix("[arena_residency peak] chunk_bytes_peak="))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no residency peak line:\n{err}"));
    assert!(peak < 1 << 20, "the stores kept their copies: chunk_bytes_peak={peak}");
}

#[test]
fn the_shapes_are_clean_under_asan() {
    let (out, _) = run("shapes", SHAPES, true, &[]);
    assert!(out.ends_with("main 200000 n4-2\n"), "{out}");
}

#[test]
fn what_another_name_reaches_survives_the_stores_under_asan() {
    for asan in [false, true] {
        let (out, _) = run("controls", CONTROLS, asan, &[]);
        assert_eq!(out, CONTROLS_OUT, "asan={asan}");
    }
}

/// Which stores take the field's own arena: one `lotus_arena_iteration_enter`
/// of a `field.arena` slot per retiring store.
#[test]
fn only_the_stores_no_other_name_reaches_into_retire() {
    let bin = harness::unique_bin("local_field_retire_ir");
    let ir = harness::build_source_ir_text(CONTROLS, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    let retiring = |name: &str| {
        let head = format!(" @{name}(");
        let body = ir
            .split("\ndefine ")
            .find(|f| f.lines().next().is_some_and(|l| l.contains(&head)))
            .unwrap_or_else(|| panic!("`{name}` in the IR"));
        let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
        body.matches("(ptr %field.arena.prev").count()
    };
    for (name, n) in [
        // the stores in the alias's scope do not retire, and the one before
        // it runs once (outside any loop)
        ("alias_before", 0),
        ("pushed_live", 1),
        ("alias_in_loop", 0),
        ("store_in_for", 0),
        ("string_into_record", 0),
        ("into_own_label", 0),
        ("pushed_whole", 0),
        ("moved_out", 0),
        ("handed_whole", 0),
        ("body_local", 2),
    ] {
        assert_eq!(retiring(name), n, "`{name}`'s retiring stores");
    }
}
