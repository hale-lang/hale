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
        let asan = run(&src, &format!("hale_array_loci_{tag}_{placement}_asan"), true, false);
        assert!(asan.ok && !asan.stderr.contains("Sanitizer"), "{what}, ASan: {}\n{}", asan.stdout, asan.stderr);
        assert_eq!(hooks(&asan.stdout).len(), 9, "{what}, ASan: {}", asan.stdout);
    }
}

#[test]
fn an_array_of_loci_is_its_owners_children() {
    check_all(r#"cs: [C; 2] = [C { name: "a" }, C { name: "b" }];"#, "array");
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
