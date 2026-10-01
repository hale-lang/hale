//! Whether a program can ever have a bus cell in flight (the
//! `bus_inert` family, F.40 phase 3), so lowering can elide its drains.
//!
//! A cell is produced only by subscriber dispatch, wire ingest into a
//! registered subscriber, the cross-pool accept handoff and
//! transport-loss dispatch, so a program is inert when nothing it
//! declares or reaches carries one of those surfaces. Two conservative
//! tiers, both structural:
//!
//! 1. The user program (every seed, imports merged) declares no topic,
//!    no perspective, and no locus with a bus block, a bindings block or
//!    an `accept`.
//! 2. It reaches no bus-surfaced stdlib declaration: it spells no
//!    mangled stdlib name (`__Std…`), and either names no `std` path at
//!    all or names none of the stdlib namespaces whose declarations reach
//!    bus surface ([`bus_tainted_namespaces`], the stdlib's column).
//!
//! What a program spells is read from the structural walk
//! (`hale_syntax::names`), never from a rendering. The test is by name,
//! and so over-approximates: a local called like a tainted namespace
//! keeps the drains, which costs the optimization, never correctness.

use std::collections::BTreeSet;

use hale_syntax::ast::{LifecycleKind, LocusMember, Program, TopDecl};
use hale_syntax::names::{for_each_spelled, for_each_spelled_in_item, Spelled};

/// The verdict for `user`, the user's program after the desugars and
/// before the stdlib is merged into it.
pub fn bus_inert(user: &Program) -> bool {
    // GH #884: every declaration, modules flattened — a topic or a bus
    // block one brace deeper is still bus surface.
    let declares_none = hale_syntax::ast::flat_decls(&user.items).all(|it| match it {
        TopDecl::Topic(_) | TopDecl::Perspective(_) => false,
        TopDecl::Locus(l) => l.members.iter().all(|m| match m {
            LocusMember::Bus(_) | LocusMember::Bindings(_) => false,
            LocusMember::Lifecycle(ld) => ld.kind != LifecycleKind::Accept,
            _ => true,
        }),
        _ => true,
    });
    if !declares_none {
        return false;
    }
    let mut names: BTreeSet<&str> = BTreeSet::new();
    let mut mangled = false;
    for_each_spelled(&user.items, &mut |s| {
        let (Spelled::Name(t) | Spelled::Text(t)) = s;
        mangled |= t.contains("__Std");
        if let Spelled::Name(n) = s {
            names.insert(n);
        }
    });
    if mangled {
        return false;
    }
    if !names.contains("std") {
        return true;
    }
    !bus_tainted_namespaces().iter().any(|ns| names.contains(ns.as_str()))
}

/// The stdlib's bus-taint column: which `std::` namespaces (the second
/// path segment) can transitively reach a bus-surfaced stdlib
/// declaration. A fixpoint over the parsed stdlib's declarations: the
/// seeds are loci with a bus block, a bindings block or an `accept`, and
/// perspectives; a declaration that names a tainted one is tainted
/// (covering a free fn that instantiates a subscriber internally,
/// `std::http::serve` → `Server`). Tainted declarations map to the
/// namespaces a program can spell through the stdlib's path renames; one
/// with no rename is reachable only through a mangled `__Std` name, which
/// tier 2 refuses outright. Computed once per process.
pub fn bus_tainted_namespaces() -> &'static [String] {
    use std::sync::OnceLock;
    static TAINT: OnceLock<Vec<String>> = OnceLock::new();
    TAINT.get_or_init(|| {
        let stdlib = hale_syntax::parse_source(hale_stdlib::AP_SOURCE)
            .expect("the bundled stdlib parses");
        let mut decls: Vec<(String, bool, BTreeSet<String>)> = Vec::new();
        for it in &stdlib.items {
            let (name, surface) = match it {
                TopDecl::Locus(l) => (
                    l.name.name.clone(),
                    l.members.iter().any(|m| match m {
                        LocusMember::Bus(_) | LocusMember::Bindings(_) => true,
                        LocusMember::Lifecycle(ld) => ld.kind == LifecycleKind::Accept,
                        _ => false,
                    }),
                ),
                TopDecl::Perspective(p) => (p.name.name.clone(), true),
                TopDecl::Fn(f) => (f.name.name.clone(), false),
                TopDecl::Type(t) => (t.name.name.clone(), false),
                TopDecl::Topic(t) => (t.name.name.clone(), false),
                TopDecl::Const(c) => (c.name.name.clone(), false),
                _ => continue,
            };
            let mut spelled = BTreeSet::new();
            for_each_spelled_in_item(it, &mut |s| {
                if let Spelled::Name(n) = s {
                    spelled.insert(n.to_string());
                }
            });
            decls.push((name, surface, spelled));
        }
        let mut tainted: BTreeSet<String> =
            decls.iter().filter(|(_, s, _)| *s).map(|(n, _, _)| n.clone()).collect();
        loop {
            let mut changed = false;
            for (n, _, spelled) in &decls {
                if !tainted.contains(n) && tainted.iter().any(|t| spelled.contains(t)) {
                    tainted.insert(n.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut ns: Vec<String> = hale_stdlib::PATH_RENAMES
            .iter()
            .filter(|(_, m)| tainted.contains(*m))
            .filter_map(|(p, _)| p.get(1).map(|s| s.to_string()))
            .collect();
        ns.sort();
        ns.dedup();
        ns
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inert(src: &str) -> bool {
        bus_inert(&hale_syntax::parse_source(src).expect("parses"))
    }

    /// The verdict reads what the program declares and the names it
    /// spells, never a rendering of it.
    #[test]
    fn the_verdict_reads_declarations_and_names() {
        assert!(bus_inert_namespace_is("log"), "std::log reaches its sink's subscription");
        assert!(inert("fn main() { println(1); }\n"), "no bus surface, no stdlib");
        assert!(inert("fn main() { std::time::sleep(1ms); }\n"), "an untainted namespace");
        assert!(!inert("fn main() { std::log::info(\"x\"); }\n"), "a tainted namespace");
        assert!(
            !inert("fn main() { let log = 1; std::time::sleep(1ms); println(log); }\n"),
            "by name: a local spelled like a tainted namespace keeps the drains"
        );
        assert!(!inert("fn main() { __StdLogSink { }; }\n"), "a mangled stdlib name");
        assert!(!inert("type P { n: Int; }\ntopic T { payload: P; }\nfn main() { }\n"), "a topic");
        assert!(
            !inert("module m {\n  type P { n: Int; }\n  topic T { payload: P; }\n}\nfn main() { }\n"),
            "a topic one module deep"
        );
    }

    fn bus_inert_namespace_is(ns: &str) -> bool {
        bus_tainted_namespaces().iter().any(|n| n == ns)
    }
}
