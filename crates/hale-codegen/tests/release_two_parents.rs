//! GH #526 F.6 (2026-09-05): one child type accepted by TWO parent
//! types. The reclaim spine found "the" release fn by child type alone
//! (`user_loci.values().find_map(...)`), so the first parent type that
//! declared `release(c: Child)` won program-wide and its body ran over
//! the other parent's `self` — the DNA core segfaulted with `Work`'s
//! release executing over `WorkSystem`'s layout. Each locus now carries
//! `__owner_release`, stored at accept dispatch from the accept'ing
//! parent TYPE, and reclaim calls through it.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
locus Child {
    params { tag: String = ""; label: String = ""; }
    contract { expose label: String; }
    run() { self.label = "ran:" + self.tag; }
}
locus ParentA {
    params { n: Int = 0; }
    accept(c: Child) { println("A accept ", c.tag); }
    release(c: Child) { self.n = self.n + 1; println("A release ", c.label); }
    run() { Child { tag: "from-A" }; }
}
locus ParentB {
    params { n: Int = 0; seen: String = ""; }
    accept(c: Child) { println("B accept ", c.tag); }
    release(c: Child) { self.n = self.n + 1; self.seen = c.label; println("B release ", c.label); }
    run() { Child { tag: "from-B" }; }
}
// A third parent accepts the same type but declares NO release: its
// child is still a flow (some parent releases the type) and reclaims
// on run-completion, with no release body called on anybody.
locus ParentC {
    params { n: Int = 0; }
    accept(c: Child) { println("C accept ", c.tag); }
    run() { Child { tag: "from-C" }; }
}
main locus App {
    params { a: ParentA = ParentA { }; b: ParentB = ParentB { }; c: ParentC = ParentC { }; }
    run() { println("a.n=", self.a.n, " b.n=", self.b.n, " b.seen=", self.b.seen, " c.n=", self.c.n); }
}
fn main() { App { }; }
"#;

#[test]
fn each_owner_type_runs_its_own_release_body() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_test_release_two_parents");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![
            "A accept from-A",
            "A release ran:from-A",
            "B accept from-B",
            "B release ran:from-B",
            "C accept from-C",
            "a.n=1 b.n=1 b.seen=ran:from-B c.n=0",
        ],
        "each owner's own release body, on its own self: {:?}",
        stdout
    );
}
