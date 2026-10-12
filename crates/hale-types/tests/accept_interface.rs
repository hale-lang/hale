//! `accept(c: I)` / `release(c: I)` naming an interface, resolved
//! statically (spec/semantics.md § Locus instantiation, "An interface
//! `accept`"): the clause admits each locus that satisfies `I` by the
//! one conformance function, the ownership graph's `accepts` holds each
//! of them, owners resolve innermost-wins over concrete and interface
//! clauses alike, an interface `release` makes each admitted locus a
//! flow, and the checker holds the body to what every admitted locus
//! has: the interface's methods.

use std::collections::BTreeMap;

use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_syntax::ast::Program;
use hale_syntax::parse_source;
use hale_types::ownership_graph::{build_ownership_graph, OwnerResolution, OwnershipGraph};
use hale_types::resolve::build_top_scope;
use hale_types::Bundle;

#[path = "support/entries.rs"]
mod entries;
use entries::{check_bundle, check_program};

fn graph(src: &str) -> OwnershipGraph {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), &prog);
    let bundle = Bundle::new(programs);
    let _ = check_bundle(&bundle);
    let (top, _diags) = build_top_scope(&bundle);
    build_ownership_graph(
        &bundle,
        &top,
        &hale_types::placement::bundle_placement(&bundle, &top),
        &hale_types::entry::entry_row(&bundle),
    )
}

fn resolution(g: &OwnershipGraph, enclosing: &str, child: &str) -> OwnerResolution {
    let hits: Vec<_> = g.sites.iter().filter(|s| s.enclosing_locus == enclosing && s.child_ty == child).collect();
    assert_eq!(hits.len(), 1, "one {enclosing} -> {child} site");
    hits[0].resolution.clone()
}

fn messages(src: &str) -> Vec<String> {
    check_program(&parse_source(src).expect("parse")).into_iter().map(|d| d.message).collect()
}

const ADMITS: &str = r#"
interface T { fn x() -> Int; }
locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }
locus B { params { v: Int = 2; } fn x() -> Int { return self.v; } }
locus Near { params { v: Int = 3; } fn x() -> String { return "no"; } }
locus Other { params { v: Int = 4; } }
locus H {
    params { n: Int = 0; }
    accept(c: T) { self.n = self.n + c.x(); }
    fn go() {
        let a = A { };
        let b = B { };
        let m = Near { };
        let o = Other { };
    }
}
fn main() { let h = H { }; h.go(); }
"#;

/// The clause admits the loci conformance admits: a method of the right
/// name with the wrong return type is no `T`, and neither is a locus
/// with no `x`.
#[test]
fn an_interface_accept_admits_each_locus_that_satisfies_it() {
    let g = graph(ADMITS);
    let accepts: Vec<&str> = g.accepts["H"].iter().map(String::as_str).collect();
    assert_eq!(accepts, ["A", "B"], "the satisfying loci, and only those");
    assert_eq!(resolution(&g, "H", "A"), OwnerResolution::SelfOwned("H".into()));
    assert_eq!(resolution(&g, "H", "B"), OwnerResolution::SelfOwned("H".into()));
    assert_eq!(resolution(&g, "H", "Near"), OwnerResolution::Orphan, "not a T: no owner");
    assert_eq!(resolution(&g, "H", "Other"), OwnerResolution::Orphan);
    assert!(messages(ADMITS).is_empty(), "the program checks: {:?}", messages(ADMITS));
}

const DEPTHS: &str = r#"
interface Shape { fn area() -> Int; }
locus Sq { params { s: Int = 1; } fn area() -> Int { return self.s * self.s; } }
locus Rect { params { w: Int = 1; } fn area() -> Int { return self.w; } }
locus Inner {
    params { n: Int = 0; }
    accept(c: Sq) { self.n = self.n + 1; }
    fn make() { let a = Sq { }; let b = Rect { }; }
}
locus Outer {
    params { n: Int = 0; }
    accept(c: Shape) { self.n = self.n + c.area(); }
    fn go() { let i = Inner { }; i.make(); let s = Sq { }; }
}
locus Mid {
    params { n: Int = 0; }
    accept(c: Shape) { self.n = self.n + 1; }
    fn make() { let s = Sq { }; }
}
locus Top {
    params { n: Int = 0; }
    accept(c: Sq) { self.n = self.n + 1; }
    fn go() { let m = Mid { }; m.make(); }
}
fn main() { let o = Outer { }; o.go(); let t = Top { }; t.go(); }
"#;

/// Innermost-wins does not ask which kind of clause the nearer ancestor
/// declares: a concrete owner below an interface owner keeps its own
/// type and lets the rest bubble, and an interface owner below a
/// concrete one keeps every type it admits.
#[test]
fn innermost_wins_across_interface_and_concrete_owners() {
    let g = graph(DEPTHS);
    assert_eq!(resolution(&g, "Inner", "Sq"), OwnerResolution::SelfOwned("Inner".into()));
    assert_eq!(resolution(&g, "Inner", "Rect"), OwnerResolution::Ancestor("Outer".into()));
    assert_eq!(resolution(&g, "Outer", "Sq"), OwnerResolution::SelfOwned("Outer".into()));
    assert_eq!(resolution(&g, "Mid", "Sq"), OwnerResolution::SelfOwned("Mid".into()), "the nearer interface owner wins");
}

/// The body is lowered once per admitted locus with the child typed as
/// that locus, so it may use only what every one of them has: a field
/// of one admitted locus is refused at the check, where the build would
/// otherwise have failed for another.
#[test]
fn the_body_may_use_only_the_interface() {
    let src = r#"
interface T { fn x() -> Int; }
locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }
locus H {
    params { n: Int = 0; }
    accept(c: T) { self.n = self.n + c.v; }
    fn go() { let a = A { }; }
}
fn main() { let h = H { }; h.go(); }
"#;
    let m = messages(src);
    assert!(m.iter().any(|m| m.contains("no field `v` on `T`")), "{m:?}");
}

/// `self.children` is one child type's collection; an interface owner's
/// children are of several types, so iterating them is refused at the
/// check. Its count is not.
#[test]
fn iterating_an_interface_owners_children_is_refused() {
    let src = r#"
interface T { fn x() -> Int; }
locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }
locus H {
    params { n: Int = 0; }
    accept(c: T) { }
    fn go() { let a = A { }; for c in self.children { self.n = self.n + 1; } }
    fn count() -> Int { return self.children.count; }
}
fn main() { let h = H { }; h.go(); }
"#;
    let m = messages(src);
    let hits: Vec<&String> = m.iter().filter(|m| m.contains("accepts through the interface `T`")).collect();
    assert_eq!(hits.len(), 1, "the loop is refused and the count is not: {m:?}");
}

/// `release(c: I)` makes each admitted locus a flow, as the snapshot's
/// flow rows say: the rows lowering reads.
#[test]
fn an_interface_release_makes_each_admitted_locus_a_flow() {
    let src = r#"
interface Job { fn w() -> Int; }
locus Small { params { v: Int = 1; } fn w() -> Int { return self.v; } run() { } }
locus Big { params { v: Int = 2; } fn w() -> Int { return self.v; } run() { } }
locus Idle { params { v: Int = 3; } run() { } }
locus Pool {
    params { n: Int = 0; }
    accept(c: Job) { }
    release (c: Job) { self.n = self.n + c.w(); }
    run() { Small { }; Big { }; Idle { }; }
}
fn main() { Pool { }; }
"#;
    let program = parse_source(src).expect("parse");
    let s = match Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) {
        Ok(s) => s,
        Err(_) => panic!("the program shapes"),
    };
    let flows = s.demand_flows().expect("the rows");
    assert!(hale_types::flows::is_flow(flows, "Small") && hale_types::flows::is_flow(flows, "Big"));
    assert!(!hale_types::flows::is_flow(flows, "Idle"), "no Job: resident");
    assert!(!hale_types::flows::is_flow(flows, "Job"), "the interface names no locus");
}

/// An alias of an interface, through a chain of aliases, is the
/// interface in every reader: the graph admits the satisfying loci, the
/// body is held to the interface's methods, and iterating the children
/// is refused.
#[test]
fn an_alias_of_an_interface_is_the_interface() {
    let src = r#"
interface Job { fn weight() -> Int; }
type Jobs = Job;
type Work = Jobs;
locus Small { params { w: Int = 3; } fn weight() -> Int { return self.w; } }
locus Plain { params { w: Int = 3; } }
locus Pool {
    params { sum: Int = 0; }
    accept(c: Work) { self.sum = self.sum + c.weight(); }
    fn go() { let s = Small { }; let p = Plain { }; }
}
fn main() { let p = Pool { }; p.go(); }
"#;
    let g = graph(src);
    let accepts: Vec<&str> = g.accepts["Pool"].iter().map(String::as_str).collect();
    assert_eq!(accepts, ["Small"], "the alias chain admits what Job does");
    assert_eq!(resolution(&g, "Pool", "Small"), OwnerResolution::SelfOwned("Pool".into()));
    assert!(messages(src).is_empty(), "{:?}", messages(src));
    let field = src.replace("c.weight()", "c.w");
    assert!(messages(&field).iter().any(|m| m.contains("no field `w` on `Job`")), "{:?}", messages(&field));
    let iter = src.replace("fn go() {", "fn go() { for c in self.children { } ");
    assert!(messages(&iter).iter().any(|m| m.contains("accepts through the interface `Job`")), "{:?}", messages(&iter));
}
