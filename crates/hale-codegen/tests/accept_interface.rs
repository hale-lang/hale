//! `accept(c: I)` / `release(c: I)` naming an interface, lowered
//! statically (spec/semantics.md § Locus instantiation, "An interface
//! `accept`"): one accept (and release) fn per locus the ownership graph
//! admits, its body lowered with the child typed as that locus, so a
//! call through the child is a direct call; one children tracker per
//! admitted locus, so each child is reclaimed by its own type's reclaim
//! and a completed flow leaves its own tracker. The run-and-check cases
//! are `tests/hale/accept_interface_test.hl` and
//! `tests/hale/accept_interface_release_test.hl`; these hold the IR shape
//! and the memory oracles.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Three children of two types, residents: accepted, summed through the
/// interface, and reclaimed by the owner's cascade, each by its own
/// type's teardown (its `dissolve` body runs).
const RESIDENTS: &str = r#"
interface T { fn x() -> Int; }
locus A {
    params { v: Int = 1; s: String = "a"; }
    fn x() -> Int { return self.v; }
    dissolve() { println(f"A {self.s} dissolved"); }
}
locus B {
    params { v: Int = 2; s: String = "b"; }
    fn x() -> Int { return self.v * 10; }
    dissolve() { println(f"B {self.s} dissolved"); }
}
locus H {
    params { n: Int = 0; }
    accept(c: T) { self.n = self.n + c.x(); }
    fn go() {
        A { v: 5, s: "a1" };
        B { v: 7, s: "b1" };
        A { v: 1, s: "a2" };
    }
    fn kids() -> Int { return self.children.count; }
    dissolve() { println(f"H dissolved n={self.n} kids={self.kids()}"); }
}
fn main() {
    let h = H { };
    h.go();
    println(f"sum={h.n}");
}
"#;

/// Flows of two types through an interface `release`, churned: each is
/// released at its run's end by its own type's release, reading its own
/// final state, and leaves its own tracker.
const FLOWS: &str = r#"
interface Job { fn weight() -> Int; }
locus Small {
    params { w: Int = 1; ran: Int = 0; tag: String = "s"; }
    fn weight() -> Int { return self.w * self.ran; }
    run() { self.ran = 1; }
}
locus Big {
    params { w: Int = 10; ran: Int = 0; tag: String = "b"; }
    fn weight() -> Int { return self.w * self.ran; }
    run() { self.ran = 2; }
}
locus Pool {
    params { released: Int = 0; weight: Int = 0; }
    accept(c: Job) { }
    release (c: Job) {
        self.released = self.released + 1;
        self.weight = self.weight + c.weight();
    }
    run() {
        let mut i = 0;
        while i < 5000 {
            Small { w: 1, tag: f"s{i}" };
            Big { w: 3, tag: f"b{i}" };
            i = i + 1;
        }
        println(f"released={self.released} weight={self.weight} tracked={self.children.count}");
    }
}
fn main() { Pool { }; }
"#;

fn run_asan(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(name);
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run the asan build");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    for marker in ["ERROR: AddressSanitizer", "ERROR: LeakSanitizer", "heap-use-after-free", "double-free"] {
        assert!(!stderr.contains(marker), "the sanitizer reported `{marker}`:\n{stderr}");
    }
    assert!(out.status.success(), "non-zero exit: {:?}\nstderr: {stderr}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// One accept fn per admitted locus, and the body's call through the
/// child is a direct call to that locus's method: no vtable, no
/// interface value built for the dispatch.
#[test]
fn each_admitted_locus_gets_its_own_accept_with_a_direct_call() {
    let bin = harness::unique_bin("hale_test_accept_iface_ir");
    let ir = harness::build_source_ir_text(RESIDENTS, &bin).expect("builds");
    let _ = std::fs::remove_file(&bin);
    for (child, method) in [("A", "@A.x"), ("B", "@B.x")] {
        let head = format!("define void @H.accept.{child}(");
        let start = ir.find(&head).unwrap_or_else(|| panic!("no `{head}` in the IR"));
        let body = &ir[start..start + ir[start..].find("\n}\n").expect("the fn ends")];
        assert!(body.contains(&format!("call i64 {method}(")), "`H.accept.{child}` calls `{method}` directly:\n{body}");
        assert!(!body.contains("vtable"), "no dispatch through a vtable:\n{body}");
    }
    assert!(!ir.contains("define void @H.accept("), "no accept fn of the interface itself");
}

/// The residents under AddressSanitizer, chunk pooling off: each child
/// is reclaimed once, by its own type's teardown, after its owner's
/// `dissolve` body, as a concrete owner's children are.
#[test]
fn residents_of_two_types_are_reclaimed_each_by_its_own_type() {
    let stdout = run_asan("hale_test_accept_iface_residents", RESIDENTS);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.first().copied(), Some("sum=76"), "{stdout}");
    assert_eq!(lines.get(1).copied(), Some("H dissolved n=76 kids=3"), "{stdout}");
    let mut children: Vec<&str> = lines[2..].to_vec();
    children.sort_unstable();
    assert_eq!(children, ["A a1 dissolved", "A a2 dissolved", "B b1 dissolved"], "{stdout}");
}

/// The churned flows under AddressSanitizer, chunk pooling off: every
/// one released through its own release fn, none left tracked.
#[test]
fn churned_flows_of_two_types_are_released_and_untracked() {
    let stdout = run_asan("hale_test_accept_iface_flows", FLOWS);
    assert!(stdout.contains("released=10000 weight=35000 tracked=0"), "{stdout}");
}
