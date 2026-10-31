//! GH #1566 follow-up: loci held in an array or a tuple param are their
//! owner's children, as a locus-typed field's one child is
//! (spec/semantics.md § the owned tree): built in the owner's arena,
//! drained before it in declaration order (element order inside), torn
//! down after its `dissolve()` body, each reclaimed once, under every
//! placement of the owner. Before, the elements were the building
//! frame's temporaries: an unplaced owner's were torn down in reverse
//! after the frame's other work, a pool-placed owner's never, and a
//! pinned owner's before the owner's own `drain()`, which then read
//! freed elements.
//!
//! Each program is built twice per placement: with the lifecycle trace
//! (held to the trace's laws, and to the order above) and with
//! AddressSanitizer (no report, no leak).

use std::process::Command;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::lifecycle::project::{self, Focus, RunPath};
use hale_types::lifecycle::trace::{self, Trace};
use hale_types::lifecycle::{ObligationKind, Point};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const PLACEMENTS: &[(&str, &str)] = &[
    ("unplaced", ""),
    ("pool", "placement { q: cooperative(pool = io); }"),
    ("pinned", "placement { q: pinned; }"),
];

/// `C` reports its hooks; `P` holds `HOLDS` and walks it in its own
/// `dissolve()`, also through a free fn named `close` (a C library's
/// name: the sanitizer build must not hand the runtime's `close(fd)` to
/// it).
fn program(holds: &str, walk: &str, placement: &str) -> String {
    format!(
        r#"locus C {{
    params {{ name: String = "c"; }}
    fn bye() {{ println(f"C.bye {{self.name}}"); }}
    drain() {{ println(f"C.drain {{self.name}}"); }}
    dissolve() {{ println(f"C.dissolve {{self.name}}"); }}
}}
fn close(c: C) {{ c.bye(); }}
locus P {{
    params {{ {holds} }}
    run() {{ std::time::sleep(20ms); }}
    drain() {{ println("P.drain"); }}
    dissolve() {{ {walk} println("P.dissolve"); }}
}}
main locus App {{
    params {{ q: P = P {{ }}; }}
    {placement}
    run() {{ std::time::sleep(100ms); println("APP DONE"); }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

struct Ran {
    stdout: String,
    stderr: String,
    ok: bool,
}

fn run(src: &str, name: &str, asan: bool, lifecycle_trace: bool) -> Ran {
    let bin = harness::unique_bin(name);
    let options = hale_codegen::BuildOptions { asan, lifecycle_trace, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let mut cmd = Command::new(&bin);
    if asan {
        cmd.env("LOTUS_NO_CHUNK_POOL", "1");
    }
    let out = cmd.output().expect("run");
    let _ = std::fs::remove_file(&bin);
    Ran {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

/// The lines of `stdout` that are hooks (`C.` / `P.`), in order.
fn hooks(stdout: &str) -> Vec<&str> {
    stdout.lines().filter(|l| l.starts_with("C.") || l.starts_with("P.")).collect()
}

/// The trace's seq of `kind` at `point` for the instance `inst`.
fn at(t: &Trace, kind: ObligationKind, point: Point, inst: u64) -> Option<u64> {
    t.events
        .iter()
        .find(|e| e.kind == kind && e.point == point && e.subject.is_some_and(|s| s.instance.raw() == inst))
        .map(|e| e.seq)
}

/// The instances of the declaration `decl` the trace saw born, in
/// birth order.
fn instances(t: &Trace, decl: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for e in &t.events {
        if e.kind == ObligationKind::Birth && e.point == Point::Completed && e.decl.as_deref() == Some(decl) {
            if let Some(s) = e.subject {
                out.push(s.instance.raw());
            }
        }
    }
    out
}

/// The owner `P` and its two `C` elements, in the trace: each born,
/// drained, dissolved and reclaimed once, the elements' drains before
/// `P`'s, `P`'s dissolve body before theirs, and their reclaims before
/// `P`'s, element order inside each step.
fn assert_owned_order(t: &Trace, what: &str) {
    let laws = trace::laws(t, true);
    assert!(laws.is_empty(), "{what}: the trace breaks its laws: {laws:?}");
    let cs = instances(t, "C");
    let ps = instances(t, "P");
    assert_eq!((cs.len(), ps.len()), (2, 1), "{what}: two elements and their owner");
    let p = ps[0];
    for kind in [ObligationKind::Drain, ObligationKind::Dissolve, ObligationKind::Reclaim] {
        for &c in &cs {
            let n = t
                .events
                .iter()
                .filter(|e| e.kind == kind && e.point == Point::Completed && e.subject.is_some_and(|s| s.instance.raw() == c))
                .count();
            assert_eq!(n, 1, "{what}: element {c}'s {kind:?} once");
        }
    }
    let seq = |k, pt, i| at(t, k, pt, i).unwrap_or_else(|| panic!("{what}: {k:?} {pt:?} of {i}"));
    let (a, b) = (cs[0], cs[1]);
    assert!(seq(ObligationKind::Drain, Point::Completed, a) < seq(ObligationKind::Drain, Point::Entered, b), "{what}: drains in element order");
    assert!(seq(ObligationKind::Drain, Point::Completed, b) < seq(ObligationKind::Drain, Point::Entered, p), "{what}: elements drain before the owner");
    assert!(seq(ObligationKind::Dissolve, Point::Completed, p) < seq(ObligationKind::Dissolve, Point::Entered, a), "{what}: the owner's dissolve body first");
    assert!(seq(ObligationKind::Dissolve, Point::Completed, a) < seq(ObligationKind::Dissolve, Point::Entered, b), "{what}: dissolves in element order");
    assert!(seq(ObligationKind::Reclaim, Point::Completed, b) < seq(ObligationKind::Reclaim, Point::Entered, p), "{what}: elements reclaimed before the owner");
}

/// The plan the lifecycle producer derives for `src` (its placement
/// table's rows, the elements' included), held over line 12's cascade:
/// every violation of `t` against it.
fn plan_violations(src: &str, tag: &str, t: &Trace) -> Vec<String> {
    let dir = harness::unique_bin(&format!("hale_array_loci_plan_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("seed dir");
    let path = dir.join("main.hl");
    std::fs::write(&path, src).expect("write the seed");
    let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false))
        .unwrap_or_else(|_| panic!("{tag}: the seed does not load"));
    let plan = snap.demand_lifecycle().unwrap_or_else(|_| panic!("{tag}: the lifecycle plan is blocked"));
    let expected = project::expected(plan, Focus::Lines(&["12"]), &RunPath::default()).unwrap_or_else(|e| panic!("{tag}: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    expected.check(t, true).into_iter().map(|v| v.to_string()).collect()
}

fn check_all(holds: &str, tag: &str) {
    let walk = "for c in self.cs { c.bye(); } close(self.cs[0]);";
    for (placement, entry) in PLACEMENTS {
        let src = program(holds, walk, entry);
        let what = format!("{tag}, {placement}");
        let traced = run(&src, &format!("hale_array_loci_{tag}_{placement}"), false, true);
        assert!(traced.ok, "{what}: {}\n{}", traced.stdout, traced.stderr);
        assert_eq!(
            hooks(&traced.stdout),
            ["C.drain a", "C.drain b", "P.drain", "C.bye a", "C.bye b", "C.bye a", "P.dissolve", "C.dissolve a", "C.dissolve b"],
            "{what}"
        );
        let t = trace::parse(&traced.stderr).unwrap_or_else(|e| panic!("{what}: the trace does not parse: {e}"));
        assert_owned_order(&t, &what);
        let off = plan_violations(&src, &format!("{tag}_{placement}"), &t);
        assert!(off.is_empty(), "{what}: the run departs from its derived plan: {off:#?}");
        // the control: the plan owes each element its teardown, so a run
        // without the elements' dissolves departs from it
        let mut without = Trace { events: t.events.clone(), rest: String::new() };
        without.events.retain(|e| !(e.kind == ObligationKind::Dissolve && e.decl.as_deref() == Some("C")));
        let off = plan_violations(&src, &format!("{tag}_{placement}_control"), &without);
        assert!(off.iter().any(|v| v.contains("C.Dissolve")), "{what}: the plan owes the elements nothing: {off:#?}");
        let asan = run(&src, &format!("hale_array_loci_{tag}_{placement}_asan"), true, false);
        assert!(asan.ok && !asan.stderr.contains("Sanitizer"), "{what}, ASan: {}\n{}", asan.stdout, asan.stderr);
        assert_eq!(hooks(&asan.stdout).len(), 9, "{what}, ASan: {}", asan.stdout);
    }
}

#[test]
fn an_array_of_loci_is_its_owners_children() {
    check_all(r#"cs: [C; 2] = [C { name: "a" }, C { name: "b" }];"#, "array");
}

/// A param with no type whose default is an array of locus literals is
/// typed by it, and holds its elements the same way.
#[test]
fn an_unascribed_array_of_loci_is_its_owners_children() {
    check_all(r#"cs = [C { name: "a" }, C { name: "b" }];"#, "inferred");
}

/// The elements a literal builds for the owner it builds, the same:
/// `P { cs: [..] }` written where `P` is built.
#[test]
fn an_array_of_loci_written_at_the_owners_literal_is_its_children() {
    for (placement, entry) in PLACEMENTS {
        let src = program("cs: [C; 2];", "for c in self.cs { c.bye(); } close(self.cs[0]);", entry)
            .replace("q: P = P { };", r#"q: P = P { cs: [C { name: "a" }, C { name: "b" }] };"#);
        let what = format!("written at the literal, {placement}");
        let traced = run(&src, &format!("hale_array_loci_lit_{placement}"), false, true);
        assert!(traced.ok, "{what}: {}\n{}", traced.stdout, traced.stderr);
        let t = trace::parse(&traced.stderr).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_owned_order(&t, &what);
        let off = plan_violations(&src, &what.replace([' ', ','], "_"), &t);
        assert!(off.is_empty(), "{what}: the run departs from its derived plan: {off:#?}");
        let asan = run(&src, &format!("hale_array_loci_lit_{placement}_asan"), true, false);
        assert!(asan.ok && !asan.stderr.contains("Sanitizer"), "{what}, ASan: {}", asan.stderr);
    }
}

/// A tuple of loci holds its parts as children, the same.
#[test]
fn a_tuple_of_loci_is_its_owners_children() {
    for (placement, entry) in PLACEMENTS {
        let src = program(r#"cs: (C, C) = (C { name: "a" }, C { name: "b" });"#, "self.cs.0.bye(); self.cs.1.bye(); close(self.cs.0);", entry);
        let what = format!("tuple, {placement}");
        let traced = run(&src, &format!("hale_tuple_loci_{placement}"), false, true);
        assert!(traced.ok, "{what}: {}\n{}", traced.stdout, traced.stderr);
        let t = trace::parse(&traced.stderr).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_owned_order(&t, &what);
        let off = plan_violations(&src, &what.replace([' ', ','], "_"), &t);
        assert!(off.is_empty(), "{what}: the run departs from its derived plan: {off:#?}");
        let asan = run(&src, &format!("hale_tuple_loci_{placement}_asan"), true, false);
        assert!(asan.ok && !asan.stderr.contains("Sanitizer"), "{what}, ASan: {}", asan.stderr);
    }
}

/// An element's own children go with it: `D`s an element holds in its
/// own array are torn down under it, before its arena.
#[test]
fn an_elements_own_array_of_loci_goes_with_it() {
    let src = r#"locus D {
    params { name: String = "d"; }
    dissolve() { println(f"D.dissolve {self.name}"); }
}
locus C {
    params { name: String = "c"; ds: [D; 1] = [D { name: "x" }]; }
    dissolve() { println(f"C.dissolve {self.name}"); }
}
locus P {
    params { cs: [C; 2] = [C { name: "a" }, C { name: "b" }]; }
    dissolve() { println("P.dissolve"); }
}
main locus App {
    params { q: P = P { }; }
    run() { println("APP DONE"); }
}
fn main() { App { }; }
"#;
    for asan in [false, true] {
        let ran = run(src, "hale_array_loci_nested", asan, false);
        assert!(ran.ok && !ran.stderr.contains("Sanitizer"), "asan={asan}: {}\n{}", ran.stdout, ran.stderr);
        let lines: Vec<&str> = ran.stdout.lines().filter(|l| l.contains("dissolve")).collect();
        assert_eq!(lines, ["P.dissolve", "C.dissolve a", "D.dissolve x", "C.dissolve b", "D.dissolve x"], "asan={asan}");
    }
}

/// An element written into the owner's array after it was built is not
/// the owner's to tear down: the instance written in keeps its own
/// owner, which tears it down once.
#[test]
fn an_element_written_in_is_left_to_its_owner() {
    let src = r#"locus C {
    params { name: String = "c"; }
    dissolve() { println(f"C.dissolve {self.name}"); }
}
locus P {
    params { cs: [C; 2] = [C { name: "a" }, C { name: "b" }]; }
    fn put(c: C) { self.cs[0] = c; }
    dissolve() { println("P.dissolve"); }
}
main locus App {
    params { q: P = P { }; e: C = C { name: "e" }; }
    run() { self.q.put(self.e); println("APP DONE"); }
}
fn main() { App { }; }
"#;
    for asan in [false, true] {
        let ran = run(src, "hale_array_loci_written_in", asan, false);
        assert!(ran.ok && !ran.stderr.contains("AddressSanitizer:"), "asan={asan}: {}\n{}", ran.stdout, ran.stderr);
        assert_eq!(ran.stdout.matches("C.dissolve e").count(), 1, "asan={asan}: {}", ran.stdout);
    }
}

/// GH #1569: a locus literal stored into an element of an array or a
/// tuple of loci, or over the whole of one, is the holder's new child —
/// WS1#4's reassign, per element: the element it replaces reclaimed
/// while the holder owns it, the new one built in the holder's arena
/// and torn down with it. It was a temporary of the storing method,
/// dissolved when the method returned and read after (ASan: SEGV), and
/// the whole-array store dissolved it twice. From a method of the
/// holder and from outside it (a fn holding a handle, a child reached
/// through `self`); a value that is not a literal is left to its owner
/// (`App`'s `e`, torn down with `App`). The value is read before the
/// store: an element the holder owns written back in (`self.cs =
/// [self.cs[1], self.cs[0]]`, `self.cs[0] = self.cs[0]`) stays its own,
/// at its new place, and only one no longer in the field is reclaimed.
#[test]
fn a_literal_stored_into_an_element_is_the_holders_child() {
    let cases: [(&str, &str, &str, &[&str]); 7] = [
        (
            "single",
            "fn swap() { self.cs[0] = C { name: \"z\" }; }",
            "self.q.swap();",
            &["C.dissolve a", "show z b", "P.dissolve", "C.dissolve z", "C.dissolve b", "C.dissolve e"],
        ),
        (
            "twice",
            "fn swap() { self.cs[0] = C { name: \"y\" }; self.cs[0] = C { name: \"z\" }; }",
            "self.q.swap();",
            &["C.dissolve a", "C.dissolve y", "show z b", "P.dissolve", "C.dissolve z", "C.dissolve b", "C.dissolve e"],
        ),
        (
            "whole",
            "fn swap() { self.cs = [C { name: \"x\" }, C { name: \"y\" }]; }",
            "self.q.swap();",
            &["C.dissolve a", "C.dissolve b", "show x y", "P.dissolve", "C.dissolve x", "C.dissolve y", "C.dissolve e"],
        ),
        (
            "indexed",
            "fn swap(i: Int) { self.cs[i] = C { name: f\"n{i}\" }; }",
            "self.q.swap(1); self.q.swap(1);",
            &["C.dissolve b", "C.dissolve n1", "show a n1", "P.dissolve", "C.dissolve a", "C.dissolve n1", "C.dissolve e"],
        ),
        (
            "outside",
            "fn noop() { }",
            "put(self.q, 1); self.q.cs = [C { name: \"m\" }, C { name: \"n\" }];",
            &["C.dissolve b", "C.dissolve a", "C.dissolve f", "show m n", "P.dissolve", "C.dissolve m", "C.dissolve n", "C.dissolve e"],
        ),
        (
            "mixed",
            "fn swap(c: C) { self.cs = [C { name: \"x\" }, c]; }",
            "self.q.swap(self.e);",
            &["C.dissolve a", "C.dissolve b", "show x e", "P.dissolve", "C.dissolve x", "C.dissolve e"],
        ),
        (
            "moved",
            "fn swap() { self.cs = [self.cs[1], self.cs[0]]; }\n    fn same() { self.cs[0] = self.cs[0]; }\n    \
             fn mix() { self.cs = [C { name: \"m\" }, self.cs[0]]; }",
            "self.q.swap(); self.q.same(); self.q.mix();",
            &["C.dissolve a", "show m b", "P.dissolve", "C.dissolve m", "C.dissolve b", "C.dissolve e"],
        ),
    ];
    for (tag, method, call, want) in cases {
        let src = format!(
            r#"locus C {{
    params {{ name: String = "c"; }}
    dissolve() {{ println(f"C.dissolve {{self.name}}"); }}
}}
locus P {{
    params {{ cs: [C; 2] = [C {{ name: "a" }}, C {{ name: "b" }}]; }}
    {method}
    fn show() {{ println(f"C.show {{self.cs[0].name}} {{self.cs[1].name}}"); }}
    dissolve() {{ println("P.dissolve"); }}
}}
fn put(h: P, i: Int) {{ h.cs[i] = C {{ name: "f" }}; }}
main locus App {{
    params {{ q: P = P {{ }}; e: C = C {{ name: "e" }}; }}
    run() {{ {call} self.q.show(); }}
}}
fn main() {{ App {{ }}; }}
"#
        );
        for asan in [false, true] {
            let ran = run(&src, &format!("hale_array_loci_literal_{tag}"), asan, false);
            assert!(ran.ok && !ran.stderr.contains("Sanitizer"), "{tag} asan={asan}: {}\n{}", ran.stdout, ran.stderr);
            let got: Vec<String> = hooks(&ran.stdout).iter().map(|l| l.replace("C.show", "show")).collect();
            assert_eq!(got, want, "{tag} asan={asan}");
        }
    }
}

/// GH #1569: the same for a tuple of loci, part by part and whole.
#[test]
fn a_literal_stored_into_a_tuple_part_is_the_holders_child() {
    let src = r#"locus C {
    params { name: String = "c"; }
    dissolve() { println(f"C.dissolve {self.name}"); }
}
locus D {
    params { n: Int = 0; }
    dissolve() { println(f"D.dissolve {self.n}"); }
}
locus P {
    params { t: (C, D) = (C { name: "a" }, D { n: 1 }); }
    fn swap() { self.t.0 = C { name: "z" }; self.t = (C { name: "w" }, D { n: 7 }); self.t.1 = D { n: 9 }; }
    fn show() { println(f"P.show {self.t.0.name} {self.t.1.n}"); }
    dissolve() { println("P.dissolve"); }
}
main locus App {
    params { q: P = P { }; }
    run() { self.q.swap(); self.q.show(); }
}
fn main() { App { }; }
"#;
    for asan in [false, true] {
        let ran = run(src, "hale_tuple_loci_literal", asan, false);
        assert!(ran.ok && !ran.stderr.contains("Sanitizer"), "asan={asan}: {}\n{}", ran.stdout, ran.stderr);
        let got: Vec<&str> = ran.stdout.lines().filter(|l| l.contains(".dissolve") || l.contains(".show")).collect();
        assert_eq!(
            got,
            [
                "C.dissolve a", "C.dissolve z", "D.dissolve 1", "D.dissolve 7", "P.show w 9",
                "P.dissolve", "C.dissolve w", "D.dissolve 9",
            ],
            "asan={asan}"
        );
    }
}

/// GH #1569: the owned-elements bitmap has a bit for every element, so
/// an array of 100 loci follows the rules above at every index: a literal
/// stored at 0, 64 and 99 is the holder's child and reclaims the element
/// it replaces, and a whole-array store reclaims, in element order, every
/// owned element it does not write back in (`self.cs[99]`, moved to 50,
/// stays the holder's). Each element is torn down once, sanitized.
#[test]
fn a_hundred_element_array_of_loci_is_tracked_at_every_index() {
    let n = 100;
    let lits: Vec<String> = (0..n).map(|k| format!("C {{ name: \"e{k}\" }}")).collect();
    let whole: Vec<String> =
        (0..n).map(|k| if k == 50 { "self.cs[99]".to_string() } else { format!("C {{ name: \"w{k}\" }}") }).collect();
    let src = format!(
        r#"locus C {{
    params {{ name: String = "c"; }}
    dissolve() {{ println(f"C.dissolve {{self.name}}"); }}
}}
locus P {{
    params {{ cs: [C; {n}] = [{lits}]; }}
    fn put() {{ self.cs[0] = C {{ name: "x0" }}; self.cs[64] = C {{ name: "x64" }}; self.cs[99] = C {{ name: "x99" }}; }}
    fn whole() {{ self.cs = [{whole}]; }}
    dissolve() {{ println("P.dissolve"); }}
}}
main locus App {{
    params {{ q: P = P {{ }}; }}
    run() {{ self.q.put(); println("P.put"); self.q.whole(); println("P.whole"); }}
}}
fn main() {{ App {{ }}; }}
"#,
        lits = lits.join(", "),
        whole = whole.join(", ")
    );
    let name = |k: usize, put: bool| match k {
        0 | 64 | 99 if put => format!("x{k}"),
        _ => format!("e{k}"),
    };
    let mut want: Vec<String> = vec!["C.dissolve e0".into(), "C.dissolve e64".into(), "C.dissolve e99".into(), "P.put".into()];
    want.extend((0..99).map(|k| format!("C.dissolve {}", name(k, true))));
    want.push("P.whole".into());
    want.push("P.dissolve".into());
    want.extend((0..n).map(|k| if k == 50 { "C.dissolve x99".to_string() } else { format!("C.dissolve w{k}") }));
    for asan in [false, true] {
        let ran = run(&src, "hale_array_loci_hundred", asan, false);
        assert!(ran.ok && !ran.stderr.contains("Sanitizer"), "asan={asan}: {}\n{}", ran.stdout, ran.stderr);
        assert_eq!(hooks(&ran.stdout), want, "asan={asan}");
    }
}

/// GH #1569: a store whose path to the holder indexes with an effect
/// (`self.ls[self.next()].cs[0] = C { }`) evaluates the index once, and
/// is the same owned store as one through a plain index: the element it
/// replaces reclaimed, the literal the holder's child.
#[test]
fn an_index_with_an_effect_on_the_holders_path_is_evaluated_once() {
    let src = r#"locus C {
    params { name: String = "c"; }
    dissolve() { println(f"C.dissolve {self.name}"); }
}
locus P {
    params { cs: [C; 2] = [C { name: "a" }, C { name: "b" }]; tag: String = "p"; }
    dissolve() { println(f"P.dissolve {self.tag}"); }
}
main locus App {
    params { ls: [P; 2] = [P { tag: "p0" }, P { tag: "p1" }]; n: Int = 0; }
    fn next() -> Int { println("C.next"); self.n = self.n + 1; return self.n - 1; }
    run() {
        self.ls[self.next()].cs[0] = C { name: "z" };
        self.ls[self.next()].cs = [C { name: "w0" }, C { name: "w1" }];
        println(f"C.read {self.ls[0].cs[0].name} {self.ls[1].cs[1].name} {self.n}");
    }
}
fn main() { App { }; }
"#;
    for asan in [false, true] {
        let ran = run(src, "hale_array_loci_effect_index", asan, false);
        assert!(ran.ok && !ran.stderr.contains("Sanitizer"), "asan={asan}: {}\n{}", ran.stdout, ran.stderr);
        assert_eq!(
            hooks(&ran.stdout),
            [
                "C.next", "C.dissolve a", "C.next", "C.dissolve a", "C.dissolve b", "C.read z w1 2",
                "P.dissolve p0", "C.dissolve z", "C.dissolve b", "P.dissolve p1", "C.dissolve w0", "C.dissolve w1",
            ],
            "asan={asan}"
        );
    }
}
