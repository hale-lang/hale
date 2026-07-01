//! Ownership-graph resolution tests — the analysis-only twin of the
//! bus-graph corpus test.
//!
//! Each small inline program is parsed + typechecked, then
//! `build_ownership_graph` resolves every instantiation site. We
//! assert the resolved `OwnedSite`s: SelfOwned (direct parent),
//! bubbling to an accepting ancestor, innermost-wins, orphan
//! detection, cross-pool edge classification, cycle-safety, and the
//! open-world (no-entry-point) bail. A tail also builds the graph over
//! a few real corpus fixtures and asserts it doesn't panic + the
//! SelfOwned edges match.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use hale_syntax::ast::Program;
use hale_syntax::parse_source;
use hale_types::ownership_graph::{
    build_ownership_graph, EdgeClass, OwnedSite, OwnerKind, OwnerResolution,
    OwnershipGraph,
};
use hale_types::resolve::build_top_scope;
use hale_types::{check_bundle, Bundle};

// --- harness -------------------------------------------------------

fn graph(src: &str) -> OwnershipGraph {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), &prog);
    let bundle = Bundle { programs };
    // Typecheck first (mirrors the corpus harness), then build off the
    // same resolved scope.
    let _ = check_bundle(&bundle);
    let (top, _diags) = build_top_scope(&bundle);
    build_ownership_graph(&bundle, &top)
}

/// The single site instantiating `child` inside `enclosing`.
fn site<'g>(
    g: &'g OwnershipGraph,
    enclosing: &str,
    child: &str,
) -> &'g OwnedSite {
    let hits: Vec<&OwnedSite> = g
        .sites
        .iter()
        .filter(|s| s.enclosing_locus == enclosing && s.child_ty == child)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one {enclosing} -> {child} site, got: {:?}",
        g.sites
            .iter()
            .map(|s| (
                s.enclosing_locus.as_str(),
                s.child_ty.as_str(),
                s.resolution.tag()
            ))
            .collect::<Vec<_>>()
    );
    hits[0]
}

// --- SelfOwned -----------------------------------------------------

#[test]
fn self_owned_direct_parent() {
    // A accepts B, A instantiates B{} → SelfOwned(A), SameTower.
    let src = r#"
        locus B { params { x: Int = 0; } }
        main locus A {
            accept(b: B) { }
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "A", "B");
    assert_eq!(s.resolution, OwnerResolution::SelfOwned("A".to_string()));
    assert_eq!(s.owner_kind, OwnerKind::DirectParent);
    assert_eq!(s.edge_class, EdgeClass::SameTower);
}

// --- Bubbling (the headline) --------------------------------------

#[test]
fn bubbling_to_accepting_grandparent() {
    // A accepts I; A instantiates B{}; B does NOT accept I; B
    // instantiates I{} → the nearest accepting ancestor is the
    // grandparent A → Ancestor(A), SameTower.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B {
            run() { I { }; }
        }
        main locus A {
            accept(i: I) { }
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("A".to_string()));
    // A is the `main locus` → provably-unique instance.
    assert_eq!(s.owner_kind, OwnerKind::SingletonConst);
    assert_eq!(s.edge_class, EdgeClass::SameTower);
}

// --- Innermost-wins -----------------------------------------------

#[test]
fn innermost_wins_self_accept_beats_ancestor() {
    // A accepts I, B ALSO accepts I, A -> B -> I{} → owner is B
    // (nearer), not A. Since B both accepts and instantiates I, this
    // is the SelfOwned(B) case (the innermost-most possible).
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B {
            accept(i: I) { }
            run() { I { }; }
        }
        main locus A {
            accept(i: I) { }
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::SelfOwned("B".to_string()));
    assert_eq!(s.owner_kind, OwnerKind::DirectParent);
}

#[test]
fn innermost_wins_nearest_ancestor_beats_farther() {
    // A accepts I, B accepts I, C does NOT; chain A -> B -> C, and
    // C instantiates I{}. The nearest accepting ancestor of C is B,
    // not A → Ancestor(B).
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus C {
            run() { I { }; }
        }
        locus B {
            accept(i: I) { }
            run() { C { }; }
        }
        main locus A {
            accept(i: I) { }
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "C", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("B".to_string()));
}

// --- PerPath ------------------------------------------------------

#[test]
fn per_path_distinct_owners_across_paths() {
    // Shared intermediary M instantiates I{}. M is reached from two
    // different acceptors: P1 (accepts I) and P2 (accepts I). The
    // owner differs by path → PerPath([P1, P2]).
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus M {
            run() { I { }; }
        }
        locus P1 {
            accept(i: I) { }
            run() { M { }; }
        }
        locus P2 {
            accept(i: I) { }
            run() { M { }; }
        }
        main locus A {
            run() { P1 { }; P2 { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "M", "I");
    assert_eq!(
        s.resolution,
        OwnerResolution::PerPath(vec!["P1".to_string(), "P2".to_string()])
    );
}

// --- Orphan -------------------------------------------------------

#[test]
fn orphan_no_accepting_ancestor() {
    // B instantiates I{}; nobody accepts I anywhere up the chain
    // (A does not accept I) → Orphan. Detected, NOT errored.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B {
            run() { I { }; }
        }
        main locus A {
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::Orphan);
    assert_eq!(s.edge_class, EdgeClass::Open);
    // Orphan is a resolved property, not a diagnostic: typecheck of the
    // same program raises no ownership error here.
}

#[test]
fn orphan_when_enclosing_is_root() {
    // The enclosing locus itself is a root (only born at fn main) and
    // nobody accepts I → Orphan.
    let src = r#"
        locus I { params { x: Int = 0; } }
        main locus A {
            run() { I { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "A", "I");
    assert_eq!(s.resolution, OwnerResolution::Orphan);
}

// --- Cross-pool edge ----------------------------------------------

#[test]
fn cross_pool_owner_placed_off_thread() {
    // Worker accepts Item and (in a method body) instantiates Mid.
    // Mid instantiates Item. Worker is placed `pinned` by the main
    // locus, Mid is same-thread → the owner (Worker) is off-thread
    // relative to the enclosing (Mid) → CrossPool.
    let src = r#"
        locus Item { params { x: Int = 0; } }
        locus Mid {
            run() { Item { }; }
        }
        locus Worker {
            accept(i: Item) { }
            run() { Mid { }; }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: pinned; }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Mid", "Item");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("Worker".to_string()));
    assert_eq!(s.edge_class, EdgeClass::CrossPool);
}

// --- Cycle-safety -------------------------------------------------

#[test]
fn cycle_safe_self_recursion_terminates() {
    // B instantiates B{} (recursion) AND I{}. A accepts I and
    // instantiates B. Resolution must terminate: I in B bubbles to A
    // despite the B<-B cycle.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B {
            run() { B { }; I { }; }
        }
        main locus A {
            accept(i: I) { }
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    // The B -> I site resolves (does not hang) to the accepting
    // ancestor A. There are two B -> B sites... actually one; assert
    // the I site.
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("A".to_string()));
}

#[test]
fn cycle_safe_pure_self_cycle_is_orphan() {
    // B only self-instantiates and instantiates I{}; nobody accepts I.
    // The climb prunes the B<-B cycle and reports Orphan without
    // looping.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B {
            run() { B { }; I { }; }
        }
        main locus A {
            run() { }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::Orphan);
}

// --- Open-world (no entry point) ----------------------------------

#[test]
fn open_world_no_entry_point_is_unanalyzable() {
    // No `fn main` and no `main locus` → the DAG is incomplete → every
    // site is Unanalyzable / Open.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B {
            accept(i: I) { }
            run() { I { }; }
        }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert!(
        matches!(s.resolution, OwnerResolution::Unanalyzable(_)),
        "expected Unanalyzable in open world, got {:?}",
        s.resolution
    );
    assert_eq!(s.edge_class, EdgeClass::Open);
}

// --- Real corpus regression ---------------------------------------

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("hale-codegen");
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

fn corpus_graph(project: &str) -> Option<OwnershipGraph> {
    let dir = examples_dir().join(project);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension().and_then(|s| s.to_str()) == Some("hl"))
                .then_some(p)
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return None;
    }
    let mut programs: BTreeMap<String, Program> = BTreeMap::new();
    for file in &files {
        let src = fs::read_to_string(file).ok()?;
        let prog = parse_source(&src).ok()?;
        programs.insert(file.to_string_lossy().into_owned(), prog);
    }
    let bundle_programs: BTreeMap<String, &Program> =
        programs.iter().map(|(k, v)| (k.clone(), v)).collect();
    let bundle = Bundle {
        programs: bundle_programs,
    };
    let _ = check_bundle(&bundle);
    let (top, _diags) = build_top_scope(&bundle);
    Some(build_ownership_graph(&bundle, &top))
}

#[test]
fn corpus_parent_child_self_owned() {
    // 02-parent-child: CoordinatorL accepts GreeterL and instantiates
    // three GreeterL{} in run() → three SelfOwned(CoordinatorL) sites.
    let Some(g) = corpus_graph("02-parent-child") else {
        eprintln!("02-parent-child fixture missing; skipping");
        return;
    };
    let greeter_sites = g.sites_for("GreeterL");
    assert!(
        !greeter_sites.is_empty(),
        "expected GreeterL instantiation sites in 02-parent-child"
    );
    for s in &greeter_sites {
        assert_eq!(s.enclosing_locus, "CoordinatorL");
        assert_eq!(
            s.resolution,
            OwnerResolution::SelfOwned("CoordinatorL".to_string()),
            "GreeterL should be self-owned by its accepting parent"
        );
        assert_eq!(s.edge_class, EdgeClass::SameTower);
    }
}

#[test]
fn corpus_walk_does_not_panic() {
    // Build the graph over every corpus project that parses. This is a
    // regression that the walk handles real programs (no panic, every
    // site classified).
    let dir = examples_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        eprintln!("examples dir missing; skipping");
        return;
    };
    let mut projects: Vec<String> = entries
        .filter_map(|e| {
            let p = e.ok()?.path();
            p.is_dir()
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    projects.sort();

    let mut total_sites = 0usize;
    for project in &projects {
        if let Some(g) = corpus_graph(project) {
            for s in &g.sites {
                // Every site carries a resolution tag — the pass never
                // leaves a site unclassified.
                let _ = s.resolution.tag();
                total_sites += 1;
            }
        }
    }
    println!("ownership-graph corpus: {total_sites} instantiation sites");
}
