//! GH #813 — a locus that contains itself by value is a located
//! check error, not a compiler crash.
//!
//! `locus Node { params { next: Node = Node { n: 1 }; } }` passed
//! `hale check` and then overflowed the compiler's own stack in
//! `lower_locus_instantiation`: every `Node` the default builds
//! leaves ITS `next` to the same default. The nesting has no floor,
//! and no call site can supply one — writing `Node { next: … }`
//! needs a `Node` to hand over, and building one asks the same
//! question again. So the declaration is the error.
//!
//! The graph is over BY-VALUE containment: an edge `L → M` when a
//! param default of `L` constructs an `M`. Two things it is
//! deliberately NOT, both pinned below:
//!
//!   * a **call** in a default (`next: Node = make()`) — lowering a
//!     call emits a call rather than inlining the callee, so the
//!     compiler terminates on it, and the checker cannot tell a
//!     factory that builds a fresh locus from an accessor handing
//!     back one somebody else owns;
//!   * a literal that supplies **every** param (`A { n: 1, m: 2 }`
//!     inside `A`'s own default) — it expands no default, so it
//!     terminates. The graph's nodes are (locus, supplied field
//!     names) for exactly this reason.

use std::collections::BTreeMap;

use hale_syntax::ast::Program;
use hale_syntax::parse_source;
use hale_types::symbol::Bundle;

fn diags_of(sources: &[(&str, &str)]) -> Vec<String> {
    let parsed: Vec<(String, Program)> = sources
        .iter()
        .map(|(name, src)| {
            (name.to_string(), parse_source(src).expect("parse"))
        })
        .collect();
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    for (name, p) in &parsed {
        programs.insert(name.clone(), p);
    }
    let bundle = Bundle::new(programs);
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.into_iter().map(|d| d.message).collect()
}

fn diags(src: &str) -> Vec<String> {
    diags_of(&[("test.hl", src)])
}

fn containment(ds: &[String]) -> Vec<&String> {
    ds.iter()
        .filter(|m| m.contains("cannot contain itself by value"))
        .collect()
}

/// The issue's program, verbatim in shape.
#[test]
fn direct_self_containment_is_reported() {
    let src = r#"
        locus Node {
            params {
                n: Int = 0;
                next: Node = Node { n: 1 };
            }
        }
        fn main() { let node = Node { }; println("n=", node.n); }
    "#;
    let ds = diags(src);
    let hits = containment(&ds);
    assert_eq!(hits.len(), 1, "exactly one report: {:?}", ds);
    assert!(
        hits[0].contains("param `next` of `Node` defaults to a `Node`"),
        "the report names the param, its locus and what it builds: {}",
        hits[0]
    );
}

/// The same cycle through two types. One report, naming the ring, at
/// the param that closes it — not one per locus on the ring.
#[test]
fn a_two_type_cycle_is_reported_with_its_ring() {
    let src = r#"
        locus Alpha {
            params {
                tag: Int = 0;
                beta: Beta = Beta { tag: 1 };
            }
        }
        locus Beta {
            params {
                tag: Int = 0;
                alpha: Alpha = Alpha { tag: 2 };
            }
        }
        fn main() { let a = Alpha { }; println("tag=", a.tag); }
    "#;
    let ds = diags(src);
    let hits = containment(&ds);
    assert_eq!(hits.len(), 1, "one report per cycle: {:?}", ds);
    assert!(
        hits[0].contains("`Alpha` → `Beta` → `Alpha`"),
        "the report spells the ring out: {}",
        hits[0]
    );
}

/// A three-type ring is the same rule; nothing about the cycle's
/// length is special-cased.
#[test]
fn a_three_type_cycle_is_reported() {
    let src = r#"
        locus A { params { t: Int = 0; b: B = B { t: 1 }; } }
        locus B { params { t: Int = 0; c: C = C { t: 1 }; } }
        locus C { params { t: Int = 0; a: A = A { t: 1 }; } }
        fn main() { let a = A { }; println("t=", a.t); }
    "#;
    let ds = diags(src);
    let hits = containment(&ds);
    assert_eq!(hits.len(), 1, "one report per cycle: {:?}", ds);
    assert!(
        hits[0].contains("`A` → `B` → `C` → `A`"),
        "the whole ring: {}",
        hits[0]
    );
}

/// The control the issue asks for: a locus holding a DIFFERENT locus
/// by value is the ordinary parent/child shape and must stay silent.
#[test]
fn a_locus_holding_a_different_locus_is_fine() {
    let src = r#"
        locus Leaf { params { tag: Int = 7; } }
        locus Holder { params { leaf: Leaf = Leaf { tag: 3 }; } }
        fn main() { let h = Holder { }; println("tag=", h.leaf.tag); }
    "#;
    let ds = diags(src);
    assert!(containment(&ds).is_empty(), "no report: {:?}", ds);
}

/// Two loci that each hold the other's *sibling* — a diamond, not a
/// cycle. Reachability alone would flag it; the walk is over edges.
#[test]
fn a_shared_child_is_not_a_cycle() {
    let src = r#"
        locus Leaf { params { tag: Int = 7; } }
        locus Left  { params { leaf: Leaf = Leaf { tag: 1 }; } }
        locus Right { params { leaf: Leaf = Leaf { tag: 2 }; } }
        locus Both {
            params {
                l: Left = Left { };
                r: Right = Right { };
            }
        }
        fn main() { let b = Both { }; println("tag=", b.l.leaf.tag); }
    "#;
    let ds = diags(src);
    assert!(containment(&ds).is_empty(), "no report: {:?}", ds);
}

/// **The pinned decision.** A param default that is a CALL returning
/// the same locus is not a containment edge.
///
/// What the code does: the program checks clean and `hale build`
/// finishes, because lowering a call emits a call — the callee's body
/// is lowered once, as a function, not inlined into the literal. The
/// built program then recurses at RUN time (`make()` builds a `Node`,
/// whose `next` default calls `make()` again) and overflows its own
/// stack, which is what any unbounded recursion does and what
/// `@no_recursion` is the contract for.
///
/// Reporting it here would mean deciding whether the callee builds a
/// fresh locus or hands back one somebody else already owns — the
/// accessor/factory question codegen answers with a whole-program
/// fixpoint (`fresh_locus_factories`) for an ownership decision, not
/// a question this per-param rule can answer from a call's spelling.
#[test]
fn a_factory_call_in_a_default_is_not_a_cycle() {
    let src = r#"
        locus Node {
            params {
                n: Int = 0;
                next: Node = make();
            }
        }
        fn make() -> Node { return Node { n: 5 }; }
        fn main() { let node = Node { n: 1 }; println("n=", node.n); }
    "#;
    let ds = diags(src);
    assert!(containment(&ds).is_empty(), "no report: {:?}", ds);
}

/// A literal that supplies every param expands no default, so it
/// terminates — and the graph's nodes carry the supplied names so
/// that it is not read as a cycle. `Probe { n: 1, m: 2 }` inside
/// `Probe`'s own default for `m` builds, runs and prints `m=1`.
#[test]
fn a_fully_supplied_literal_in_its_own_default_is_not_a_cycle() {
    let src = r#"
        locus Probe {
            params {
                n: Int = 0;
                m: Int = Probe { n: 1, m: 2 }.n;
            }
        }
        fn main() { let p = Probe { }; println("m=", p.m); }
    "#;
    let ds = diags(src);
    assert!(containment(&ds).is_empty(), "no report: {:?}", ds);
}

/// A locus literal nested inside another of the SAME type, written
/// out in source, is bounded by the source that spells it. Reached
/// here through an interface-typed field, the only way to write the
/// shape (a same-type field with no default cannot be supplied, and
/// with one it is the cycle above).
#[test]
fn same_type_nesting_written_out_is_fine() {
    let src = r#"
        interface Shape { fn area() -> Int; }
        locus Dot { params { n: Int = 1; } fn area() -> Int { return self.n; } }
        locus Box {
            params { inner: Shape; n: Int = 0; }
            fn area() -> Int { return self.n + self.inner.area(); }
        }
        fn main() {
            let b = Box { inner: Box { inner: Dot { n: 1 }, n: 2 }, n: 4 };
            println("area=", b.area());
        }
    "#;
    let ds = diags(src);
    assert!(containment(&ds).is_empty(), "no report: {:?}", ds);
}

/// The gating the issue asks for, and how it is obtained: a cycle
/// that crosses files has no edge in a single file's check, because
/// the sibling's `locus` declaration is not in the bundle. Checking
/// the seed together supplies it.
#[test]
fn a_cycle_across_files_needs_the_whole_seed() {
    let alpha = r#"
        locus Alpha {
            params { tag: Int = 0; beta: Beta = Beta { tag: 1 }; }
        }
        fn main() { let a = Alpha { }; println("tag=", a.tag); }
    "#;
    let beta = r#"
        locus Beta {
            params { tag: Int = 0; alpha: Alpha = Alpha { tag: 2 }; }
        }
    "#;
    let alone = diags_of(&[("alpha.hl", alpha)]);
    assert!(
        containment(&alone).is_empty(),
        "one file of a multi-file seed stays permissive: {:?}",
        alone
    );
    let together = diags_of(&[("alpha.hl", alpha), ("beta.hl", beta)]);
    assert_eq!(
        containment(&together).len(),
        1,
        "the whole seed sees the cycle: {:?}",
        together
    );
}

/// The rule reads params inside a `module`, which is where a larger
/// seed puts its types.
#[test]
fn self_containment_inside_a_module_is_reported() {
    let src = r#"
        module tree {
            locus Node {
                params { n: Int = 0; next: Node = Node { n: 1 }; }
            }
        }
        fn main() { println("ok"); }
    "#;
    let ds = diags(src);
    assert_eq!(containment(&ds).len(), 1, "reported: {:?}", ds);
}
