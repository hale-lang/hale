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
    build_ownership_graph, EdgeClass, ExpandedLiteral, OwnedSite, OwnerKind, OwnerResolution,
    OwnershipGraph,
};
use hale_types::resolve::build_top_scope;
use hale_types::{check_bundle, Bundle};

// --- harness -------------------------------------------------------

fn graph(src: &str) -> OwnershipGraph {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), &prog);
    let bundle = Bundle::new(programs);
    // Typecheck first (mirrors the corpus harness), then build off the
    // same resolved scope.
    let _ = check_bundle(&bundle);
    let (top, _diags) = build_top_scope(&bundle);
    build_ownership_graph(&bundle, &top, &hale_types::placement::bundle_placement(&bundle, &top), &hale_types::entry::entry_row(&bundle))
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
fn a_child_born_in_a_placed_owners_body_is_same_tower() {
    // Worker accepts Item and (in a method body) instantiates Mid.
    // Mid instantiates Item. Worker is placed on pool `io`, and Mid,
    // born in Worker's body, runs where Worker runs: the owner and the
    // enclosing locus share a thread → SameTower. The legacy label
    // read only the root's entries, called Mid same-thread and Worker
    // cross-pool, and gave CrossPool (F.40 phase 3, P1 3 of 6: a
    // locus born in a body runs in its enclosing scope's domain).
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
            placement { w: cooperative(pool = io); }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Mid", "Item");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("Worker".to_string()));
    assert_eq!(s.edge_class, EdgeClass::SameTower);
}

// --- The table's edge classes (F.40 phase 3, P1 3 of 6) -------------
//
// Each edge compares the domains of the enclosing locus's instances
// with their owners', read from the placement table, per instance (the
// placement correspondence's § 2.3).

/// O-1: an enclosing locus nested under a pinned root field runs on that
/// field's thread, so a child it bubbles to the root (on main) is a
/// cross-pool birth, and lowering posts it. The legacy label called the
/// nested locus same-thread, and lowering allocated the child in the
/// root's arena from the pinned thread.
#[test]
fn a_nested_enclosing_under_a_pinned_field_is_cross_pool() {
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus Worker {
            fn spawn() { I { }; }
        }
        locus Owner {
            params { w: Worker = Worker { }; }
        }
        main locus App {
            params { o: Owner = Owner { }; }
            placement { o: pinned; }
            accept(c: I) { }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Worker", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("App".to_string()));
    assert_eq!(s.edge_class, EdgeClass::CrossPool);
    let plans = g.bubble_plans();
    let key = ("Worker".to_string(), "I".to_string());
    assert_eq!(plans.crosspool.get(&key), Some(&"App".to_string()), "lowering posts the birth");
    assert!(!plans.singleton.contains_key(&key), "and never allocates it from the pinned thread");
}

/// O-2: an enclosing locus nested under a root field on pool `io`,
/// bubbling to that field (on `io` too), shares its owner's thread:
/// SameTower, so lowering threads the owner pointer instead of leaving
/// the child transient. The legacy labels gave CrossPool.
#[test]
fn a_nested_enclosing_on_its_owners_pool_is_same_tower() {
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus Worker {
            fn spawn() { I { }; }
        }
        locus Hub {
            params { w: Worker = Worker { }; }
            accept(c: I) { }
        }
        main locus App {
            params { h: Hub = Hub { }; }
            placement { h: cooperative(pool = io); }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Worker", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("Hub".to_string()));
    assert_eq!(s.edge_class, EdgeClass::SameTower);
    let key = ("Worker".to_string(), "I".to_string());
    assert_eq!(g.bubble_plans().nonsingleton.get(&key), Some(&"Hub".to_string()));
}

/// The pairing is per instance: two owners in two domains, each nesting
/// its own enclosing instance, are each on their own instance's thread.
/// SameTower, though the types' domain sets are {main, io} on both
/// sides.
#[test]
fn two_owners_in_two_domains_each_own_their_nested_children() {
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus Worker {
            fn spawn() { I { }; }
        }
        locus Hub {
            params { w: Worker = Worker { }; }
            accept(c: I) { }
        }
        main locus App {
            params { h1: Hub = Hub { }; h2: Hub = Hub { }; }
            placement { h2: cooperative(pool = io); }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Worker", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("Hub".to_string()));
    assert_eq!(s.edge_class, EdgeClass::SameTower);
}

/// O-7: an adapter in the root's `bindings { }` runs on its own thread,
/// and so does its tower. As an owner, a child its nested locus bubbles
/// to it is born on that thread: SameTower, each instance paired with
/// its own owner row. A birth in the adapter itself is self-owned.
/// These graph rows can be derived even though admission rejects an
/// adapter with `accept()` under rule 6; the graph describes ownership,
/// and the checker separately judges the pinned lifecycle restriction.
#[test]
fn an_adapter_edge_runs_on_the_adapters_thread() {
    let src = r#"
        type Wire { n: Int; }
        topic Beat { payload: Wire; subject: "beat"; }
        locus I { params { x: Int = 0; } }
        locus Worker {
            fn spawn() { I { }; }
        }
        locus Probe {
            params { w: Worker = Worker { }; }
            accept(c: I) { }
            fn send(subject: String, bytes: Bytes) { I { }; }
        }
        main locus App {
            bindings { Beat: Probe { }; }
            bus { publish Beat; }
            accept(c: I) { }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Worker", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("Probe".to_string()));
    assert_eq!(s.edge_class, EdgeClass::SameTower);
    let own = site(&g, "Probe", "I");
    assert_eq!(own.resolution, OwnerResolution::SelfOwned("Probe".to_string()));
}

/// O-3, as U-1 decides: one enclosing locus with an instance on its
/// owner's thread and one off it keeps its resolved owner, and its edge
/// is `Mixed`, naming every instance and its domain; the plan is the
/// per-instance one, never the transient fallback.
#[test]
fn a_mixed_enclosing_keeps_its_owner() {
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus Worker {
            fn spawn() { I { }; }
        }
        locus Owner {
            params { w: Worker = Worker { }; }
        }
        main locus App {
            params { a: Worker = Worker { }; o: Owner = Owner { }; }
            placement { o: pinned; }
            accept(c: I) { }
        }
        fn main() { App { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "Worker", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("App".to_string()));
    assert_eq!(s.owner_kind, OwnerKind::SingletonConst);
    assert_eq!(s.edge_class, EdgeClass::Mixed);
    assert_eq!(
        s.instances,
        ["App.a on main", "App.o.w on the pinned thread of App.o", "the owner App on main"],
    );
    let plans = g.bubble_plans();
    let key = ("Worker".to_string(), "I".to_string());
    let plan = plans.mixed.get(&key).expect("a mixed plan");
    assert_eq!((plan.owner.as_str(), plan.singleton), ("App", true));
    assert!(
        !plans.singleton.contains_key(&key) && !plans.crosspool.contains_key(&key) && !plans.nonsingleton.contains_key(&key),
        "the mixed plan is the edge's only plan"
    );
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

// --- #2b forwarding sets ------------------------------------------

#[test]
fn forwarding_set_bubbling_grandparent_carries_child() {
    // Non-singleton acceptor `A` (two instances born under `Root`),
    // reached through intermediary `B` that does NOT accept `I`. The
    // site resolves to `Ancestor(A)` with `OwnerKind::Ancestor` (NOT
    // SingletonConst) → `B` must carry `__owner_for_I`. `A` is the
    // owner (accepts I), so it is excluded from the set.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B { run() { I { }; } }
        locus A {
            accept(i: I) { }
            run() { B { }; }
        }
        main locus Root { run() { A { }; A { }; } }
        fn main() { Root { }; }
    "#;
    let g = graph(src);
    // Precondition: the site is the non-singleton Ancestor case.
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("A".to_string()));
    assert_eq!(s.owner_kind, OwnerKind::Ancestor);
    assert_eq!(s.edge_class, EdgeClass::SameTower);

    let fset = g.compute_forwarding_sets();
    assert_eq!(
        fset.get("B").cloned().unwrap_or_default(),
        ["I".to_string()].into_iter().collect(),
        "enclosing B must carry __owner_for_I; got {:?}",
        fset
    );
    // A is the owner (excluded); Root neither carries nor accepts I.
    assert!(!fset.contains_key("A"), "owner A must not carry: {:?}", fset);
    assert!(!fset.contains_key("Root"), "Root must not carry: {:?}", fset);
}

#[test]
fn forwarding_set_three_level_chain_threads_both_intermediaries() {
    // Root -> W{} (x2, non-singleton) ; W accepts I ; W -> A -> B -> I{}.
    // The nearest acceptor of B is W. Every intermediary between B and W
    // (i.e. B and A, W excluded) must carry `__owner_for_I`.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B { run() { I { }; } }
        locus A { run() { B { }; } }
        locus W {
            accept(i: I) { }
            run() { A { }; }
        }
        main locus Root { run() { W { }; W { }; } }
        fn main() { Root { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert_eq!(s.resolution, OwnerResolution::Ancestor("W".to_string()));
    assert_eq!(s.owner_kind, OwnerKind::Ancestor);

    let fset = g.compute_forwarding_sets();
    let want: std::collections::BTreeSet<String> =
        ["I".to_string()].into_iter().collect();
    assert_eq!(fset.get("A").cloned().unwrap_or_default(), want, "A: {:?}", fset);
    assert_eq!(fset.get("B").cloned().unwrap_or_default(), want, "B: {:?}", fset);
    assert!(!fset.contains_key("W"), "owner W excluded: {:?}", fset);
    assert!(!fset.contains_key("Root"), "Root excluded: {:?}", fset);
}

#[test]
fn forwarding_set_singleton_owner_yields_nothing() {
    // The #2 singleton case: `A` is a `main locus` (SingletonConst) →
    // stays on #2's global-constant path, so NO threading fields.
    let src = r#"
        locus I { params { x: Int = 0; } }
        locus B { run() { I { }; } }
        main locus A {
            accept(i: I) { }
            run() { B { }; }
        }
        fn main() { A { }; }
    "#;
    let g = graph(src);
    let s = site(&g, "B", "I");
    assert_eq!(s.owner_kind, OwnerKind::SingletonConst);

    let fset = g.compute_forwarding_sets();
    assert!(
        fset.is_empty(),
        "singleton owner must thread nothing; got {:?}",
        fset
    );
}

// --- The cross-pool spawn law (F.40 phase 3, C7) -----------------
//
// A cross-pool `I { }` is fire-and-forget: born on the owner's thread,
// it may only be a bare statement. Until C7 lowering was the only
// evaluator (`hale check` passed a value use; `hale build` refused it
// without a span). The law reads the graph's cross-pool bubble plan
// (`hale_types::lowering_laws`). The programs are spelled without raw
// strings so the corpus does not harvest them.

/// `Driver` on pool `workers` writes `Ship { }`; `World`, a singleton
/// on main, accepts it: a cross-pool bubble. `body` is `Driver.run()`'s.
fn crosspool_src(body: &str) -> String {
    [
        "locus Ship { params { hull: Int = 0; } contract { expose hull: Int; } }\n",
        "locus Box { params { s: Ship; } }\n",
        "locus Holder { params { s: Ship = Ship { hull: 1 }; } }\n",
        "fn keep(s: Ship) { }\n",
        "locus Driver {\n    run() {\n",
        body,
        "\n    }\n}\n",
        "main locus World {\n",
        "    params { driver: Driver = Driver { }; }\n",
        "    placement { driver: cooperative(pool = workers); }\n",
        "    accept(s: Ship) { }\n",
        "    run() { }\n",
        "}\n",
        "fn main() { World { }; }\n",
    ]
    .concat()
}

const FIRE_AND_FORGET: &str = "cross-pool spawn `Ship{ }` is fire-and-forget: the instance is created on \
                               `World`'s thread and cannot be used here";

fn crosspool_errors(body: &str) -> Vec<(String, String)> {
    let src = crosspool_src(body);
    let prog = parse_source(&src).expect("parse failed");
    hale_types::check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message.clone(), src[d.span.start.as_usize()..d.span.end.as_usize()].to_string()))
        .collect()
}

#[test]
fn a_cross_pool_spawn_used_as_a_value_is_refused_at_the_literal() {
    for (body, what) in [
        ("        let s = Ship { hull: 7 };", "let-bound"),
        ("        keep(Ship { hull: 7 });", "an argument"),
        ("        Box { s: Ship { hull: 7 } };", "a field of a bare literal"),
    ] {
        let errs = crosspool_errors(body);
        assert_eq!(errs.len(), 1, "{what}: the law's refusal and nothing else: {errs:?}");
        let (msg, at) = &errs[0];
        assert!(msg.starts_with(FIRE_AND_FORGET), "{what}: {msg}");
        assert_eq!(at, "Ship { hull: 7 }", "{what}: located at the literal");
    }
}

#[test]
fn a_bare_cross_pool_spawn_is_clean() {
    let errs = crosspool_errors("        Ship { hull: 7 };");
    assert!(errs.is_empty(), "a bare statement is the legal spelling: {errs:?}");
}

/// The shape lowering refused alone until C3 rest: `Driver` spawns
/// `Ship` itself (bare, legal), so the plan holds (Driver, Ship);
/// `Holder`'s params default builds a `Ship` too, and lowering expands
/// it under `Driver`'s self, where that entry applies. The graph keys the
/// literal by `Holder` and gives it its context, `Driver`
/// (`expansions`), so the law refuses it at the literal in the
/// default.
const FIRE_AND_FORGET_DEFAULT: &str = "cross-pool spawn `Ship{ }` is fire-and-forget: it is the default of \
                                       `Holder`'s param `s`, which is built in `Driver` (a `Holder` built there \
                                       leaves `s` to its default), so the instance is created on `World`'s \
                                       thread and cannot be the field's value";

#[test]
fn a_cross_pool_spawn_in_another_locus_default_is_refused_at_the_default() {
    let g = graph(&crosspool_src("        Ship { hull: 7 };\n        Holder { };"));
    let crosspool = g.bubble_plans().crosspool;
    assert!(crosspool.contains_key(&("Driver".to_string(), "Ship".to_string())));
    let contexts: Vec<(String, String)> = g
        .expansions(&crosspool, &Default::default())
        .into_iter()
        .filter_map(|e| match e.literal {
            ExpandedLiteral::Owned(i) => Some((
                format!("{}.{}", g.declarations[g.sites[i].enclosing_decl].name, g.sites[i].child_ty),
                g.declarations[e.context].name.clone(),
            )),
            ExpandedLiteral::Free(_) | ExpandedLiteral::Other(_) => None,
        })
        .collect();
    assert!(contexts.contains(&("Holder.Ship".to_string(), "Driver".to_string())), "{contexts:?}");
    let errs = crosspool_errors("        Ship { hull: 7 };\n        Holder { };");
    assert_eq!(errs.len(), 1, "the law's refusal and nothing else: {errs:?}");
    let (msg, at) = &errs[0];
    assert!(msg.starts_with(FIRE_AND_FORGET_DEFAULT), "{msg}");
    assert_eq!(at, "Ship { hull: 1 }", "located at the literal in the default");
}

/// Two defaults deep: `Driver` builds a `Dock`, whose default builds the
/// `Holder` whose default builds the `Ship`; both expand under `Driver`.
#[test]
fn a_cross_pool_spawn_two_defaults_deep_is_refused_at_the_default() {
    let src = crosspool_src("        Ship { hull: 7 };\n        Dock { };")
        .replace("fn keep(", "locus Dock { params { h: Holder = Holder { }; } }\nfn keep(");
    let prog = parse_source(&src).expect("parse failed");
    let errs: Vec<(String, String)> = hale_types::check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message.clone(), src[d.span.start.as_usize()..d.span.end.as_usize()].to_string()))
        .collect();
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(FIRE_AND_FORGET_DEFAULT), "{}", errs[0].0);
    assert_eq!(errs[0].1, "Ship { hull: 1 }");
}

/// A default no cross-pool context expands is clean: the `Holder` that
/// `Driver` builds supplies `s` (a value it was handed), and the one
/// `World` builds expands the default on `World`'s own thread, where
/// `World` accepts the `Ship`.
#[test]
fn a_default_no_cross_pool_context_expands_is_clean() {
    let src = crosspool_src("        Ship { hull: 7 };")
        .replace("locus Driver {\n", "locus Driver {\n    params { got: Ship; }\n    fn hold() { Holder { s: self.got }; }\n")
        .replace("    run() { }\n}\n", "    run() { Holder { }; }\n}\n")
        .replace("Driver { }", "Driver { got: Ship { } }");
    let prog = parse_source(&src).expect("parse failed");
    let errs: Vec<String> = hale_types::check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error() && d.message.contains("fire-and-forget"))
        .map(|d| d.message)
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
}

/// The review of #1351: an argument's default is lowered at each call
/// that leaves the argument out, in the caller's scope and under the
/// caller's locus. `src` adds `decls` before `Driver` and runs `body` in
/// `Driver.run()`; the errors, each with the text it is located at.
fn arg_default_errors(decls: &str, body: &str) -> Vec<(String, String)> {
    let src = crosspool_src(body).replace("locus Driver {\n", &format!("{decls}locus Driver {{\n"));
    let prog = parse_source(&src).expect("parse failed");
    hale_types::check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message.clone(), src[d.span.start.as_usize()..d.span.end.as_usize()].to_string()))
        .collect()
}

const TAKE: &str = "fn take(s: Ship = Ship { hull: 1 }) -> Int { return s.hull; }\n";

fn fire_and_forget_arg(callee: &str, param: &str) -> String {
    format!(
        "cross-pool spawn `Ship{{ }}` is fire-and-forget: it is the default of `{callee}`'s argument `{param}`, \
         expanded here, in `Driver`"
    )
}

/// The review's shape: `take()` leaves `s` to `Ship { hull: 1 }`, which
/// lowering builds in `Driver`, where (Driver, Ship) is a post. Refused
/// at the call; supplying the argument expands nothing.
#[test]
fn a_cross_pool_spawn_in_a_fn_argument_default_is_refused_at_the_call_that_omits_it() {
    let errs = arg_default_errors(TAKE, "        Ship { hull: 7 };\n        take();");
    assert_eq!(errs.len(), 1, "the law's refusal and nothing else: {errs:?}");
    assert!(errs[0].0.starts_with(&fire_and_forget_arg("take", "s")), "{}", errs[0].0);
    assert_eq!(errs[0].1, "take()", "located at the call that omits the argument");
    // `Driver` is handed a `Ship` built on `World`'s thread.
    let src = crosspool_src("        Ship { hull: 7 };\n        take(self.got);")
        .replace("locus Driver {\n", &format!("{TAKE}locus Driver {{\n    params {{ got: Ship; }}\n"))
        .replace("Driver { }", "Driver { got: Ship { } }");
    let prog = parse_source(&src).expect("parse failed");
    let errs: Vec<String> =
        hale_types::check_program(&prog).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errs.is_empty(), "a call that supplies the argument expands no default: {errs:?}");
}

/// A method's default, called on `self` and on a receiver: each call is
/// judged in `Driver`, and the default's literal is never judged under
/// the locus that declares it.
#[test]
fn a_cross_pool_spawn_in_a_method_argument_default_is_refused_at_each_call_that_omits_it() {
    let tool = "locus Tool { fn use_it(s: Ship = Ship { hull: 2 }) -> Int { return s.hull; } }\n";
    let errs = arg_default_errors(tool, "        Ship { hull: 7 };\n        let t = Tool { };\n        t.use_it();");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(&fire_and_forget_arg("Tool.use_it", "s")), "{}", errs[0].0);
    assert_eq!(errs[0].1, "t.use_it()");
    // On `self`: `Driver`'s own method, its default expanded in `run()`.
    let src = crosspool_src("        Ship { hull: 7 };\n        self.own();")
        .replace("    run() {\n", "    fn own(s: Ship = Ship { hull: 3 }) -> Int { return s.hull; }\n    run() {\n");
    let prog = parse_source(&src).expect("parse failed");
    let errs: Vec<(String, String)> = hale_types::check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message.clone(), src[d.span.start.as_usize()..d.span.end.as_usize()].to_string()))
        .collect();
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(&fire_and_forget_arg("Driver.own", "s")), "{}", errs[0].0);
    assert_eq!(errs[0].1, "self.own()");
}

/// Transitive: `outer()` leaves `n` to `inner()`, which leaves `s` to the
/// `Ship`; and a params default that calls `inner()` reaches it too. Each
/// is refused at what `Driver`'s body writes.
#[test]
fn a_cross_pool_spawn_reached_through_a_chain_of_defaults_is_refused_at_the_callers_root() {
    let chain = "fn inner(s: Ship = Ship { hull: 1 }) -> Int { return s.hull; }\n\
                 fn outer(n: Int = inner()) -> Int { return n; }\n\
                 locus Counter { params { n: Int = inner(); } }\n";
    let errs = arg_default_errors(chain, "        Ship { hull: 7 };\n        outer();");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(&fire_and_forget_arg("inner", "s")), "{}", errs[0].0);
    assert_eq!(errs[0].1, "outer()");
    let errs = arg_default_errors(chain, "        Ship { hull: 7 };\n        Counter { };");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(&fire_and_forget_arg("inner", "s")), "{}", errs[0].0);
    assert_eq!(errs[0].1, "Counter { }");
    // `outer(5)` supplies `n`, so `inner()` is never expanded.
    let errs = arg_default_errors(chain, "        Ship { hull: 7 };\n        outer(5);");
    assert!(errs.is_empty(), "{errs:?}");
    // An argument default that builds a `Holder` leaving `s`: `Holder`'s
    // params default is expanded at the call too, and refused where a
    // params default is, at its literal, built in `Driver`.
    let errs = arg_default_errors(
        "fn hold(h: Holder = Holder { }) { }\n",
        "        Ship { hull: 7 };\n        hold();",
    );
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(FIRE_AND_FORGET_DEFAULT), "{}", errs[0].0);
    assert_eq!(errs[0].1, "Ship { hull: 1 }");
}

/// A `const`'s value is lowered at every read, under the reader's locus,
/// which no row names: a literal it builds, or one its defaults build, is
/// refused at the const whenever some locus posts the child. Where none
/// does, it is clean.
#[test]
fn a_cross_pool_spawn_in_a_const_value_is_refused_at_the_const() {
    let per_use = "cross-pool spawn `Ship{ }` is fire-and-forget: it is built ";
    let errs = arg_default_errors(
        "const S: Ship = Ship { hull: 1 };\n",
        "        Ship { hull: 7 };\n        println(S.hull);",
    );
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].0.starts_with(&format!("{per_use}here, in a `const`'s value")), "{}", errs[0].0);
    assert_eq!(errs[0].1, "Ship { hull: 1 }");
    let errs = arg_default_errors("const H: Holder = Holder { };\n", "        Ship { hull: 7 };\n        let h = H;");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(
        errs[0].0.starts_with(&format!("{per_use}through the defaults this leaves, in a `const`'s value")),
        "{}",
        errs[0].0
    );
    assert_eq!(errs[0].1, "Holder { }");
    // No locus posts `Ship`: nothing crosses wherever the const is read.
    let errs = arg_default_errors("const S: Ship = Ship { hull: 1 };\n", "        println(S.hull);");
    assert!(errs.is_empty(), "{errs:?}");
}

/// A generic fn's default is never expanded: a generic call supplies
/// every argument (the checker's arity rule), so there is no expansion
/// before specialization for the law to place.
#[test]
fn a_generic_fn_call_leaves_no_argument_to_its_default() {
    let errs = arg_default_errors(
        "fn tag<T>(x: T, s: Ship = Ship { hull: 1 }) -> Int { return s.hull; }\n",
        "        Ship { hull: 7 };\n        tag(3);",
    );
    assert!(errs.iter().any(|(m, at)| m == "generic fn `tag` takes 2 arguments, got 1" && at == "tag"), "{errs:?}");
}

/// The same default from two callers: `Driver` (pool `workers`) crosses,
/// `World` (where `Ship` is accepted) does not. One refusal, at
/// `Driver`'s call; and a caller with no post for `Ship` is clean.
#[test]
fn an_argument_default_is_refused_only_at_the_caller_that_crosses() {
    let src = crosspool_src("        Ship { hull: 7 };\n        take();")
        .replace("locus Driver {\n", &format!("{TAKE}locus Driver {{\n"))
        .replace("    run() { }\n}\n", "    run() { take(); }\n}\n");
    let prog = parse_source(&src).expect("parse failed");
    let errs: Vec<(String, usize)> = hale_types::check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message.clone(), d.span.start.as_usize()))
        .collect();
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0].1, src.find("        take();").unwrap() + 8, "at `Driver`'s call: {errs:?}");
    // No bare `Ship` in `Driver`: the plan has no (Driver, Ship) post,
    // and lowering births the default's `Ship` where it stands.
    assert!(arg_default_errors(TAKE, "        take();").is_empty());
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
    let bundle = Bundle::new(bundle_programs);
    let _ = check_bundle(&bundle);
    let (top, _diags) = build_top_scope(&bundle);
    Some(build_ownership_graph(&bundle, &top, &hale_types::placement::bundle_placement(&bundle, &top), &hale_types::entry::entry_row(&bundle)))
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

#[test]
fn corpus_threads_nothing_2b_inert() {
    // #2b inertness gate: the corpus has ZERO non-singleton same-tower
    // Ancestor sites, so `compute_forwarding_sets` is empty on every
    // project → no `__owner_for_<I>` fields, no birth-threading, no
    // non-singleton bubble stitch. This is what makes bubbling ON vs
    // `LOTUS_NO_OWNERSHIP_BUBBLE=1` byte-identical on the corpus.
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

    for project in &projects {
        if let Some(g) = corpus_graph(project) {
            let fset = g.compute_forwarding_sets();
            assert!(
                fset.is_empty(),
                "#2b must be inert on the corpus, but project `{project}` \
                 threads: {:?}",
                fset
            );
        }
    }
}

/// Outside review of #1276, finding 2: the envelope's graph is built
/// over a bundle that carries the build's import renames, so a
/// qualified imported type in `accept(c: lib::Child)` resolves to the
/// locus the merge declared it as. Lowering reads `accepts` as
/// authoritative for a locus with a row, so an empty set here meant
/// the declared `accept` never ran.
#[test]
fn resolved_graph_resolves_an_imported_accept_type() {
    let src = r#"
        locus ImportedChild { params { id: Int = 0; } }
        locus Parent {
            params { count: Int = 0; }
            accept(c: lib::Child) { self.count = self.count + 1; }
            run() { lib::Child { id: 1 }; }
        }
        fn main() { Parent { }; }
    "#;
    let prog = parse_source(src).expect("parse failed");
    let renames = vec![(vec!["lib".to_string(), "Child".to_string()], "ImportedChild".to_string())];
    let resolved = hale_types::resolved::resolve_program(&prog, &[], &renames, None, None, &hale_types::form_rows::FormRows::default(), &hale_types::binding_rows::BindingRows::default(), &hale_types::placement::PlacementTable::default(), &hale_types::typed_bodies::TypedBodies::default())
        .expect("resolve");
    let want: std::collections::BTreeSet<String> = ["ImportedChild".to_string()].into();
    assert_eq!(resolved.ownership.accepts.get("Parent"), Some(&want));
    assert_eq!(resolved.bundle().import_renames, renames, "the bundle view carries them too");
    let birth = site(&resolved.ownership, "Parent", "ImportedChild");
    assert_eq!(birth.child_key.as_deref(), Some("ImportedChild"));
    assert_eq!(birth.resolution, OwnerResolution::SelfOwned("Parent".into()));
    let declaration = &resolved.ownership.declarations[birth.child_decl.unwrap()];
    assert_eq!(declaration.name, "ImportedChild");
    assert!(declaration.id.is_some(), "lowering's bundle carries the minted snapshot");
    let child = resolved.merged.items.iter().find_map(|d| match d {
        hale_syntax::ast::TopDecl::Locus(l) if l.name.name == "ImportedChild" => Some(l.id),
        _ => None,
    }).unwrap();
    assert_eq!(declaration.id, resolved.snapshot.site_id(child));
}

/// An alias is a birth of its target declaration, including in a free
/// function. The written alias is never an ownership or bubbling key.
#[test]
fn aliased_births_use_the_resolved_child() {
    let g = graph(r#"
        locus Kid { }
        type First = Kid;
        type Held = First;
        main locus App {
            accept(k: Held) { }
            run() { Held { }; }
        }
        fn make() { Held { }; }
        fn main() { App { }; make(); }
    "#);
    let birth = site(&g, "App", "Kid");
    assert_eq!(birth.child_key.as_deref(), Some("Kid"));
    assert_eq!(birth.resolution, OwnerResolution::SelfOwned("App".into()));
    assert_eq!(g.declarations[birth.child_decl.unwrap()].name, "Kid");
    let free = g.free_fn_sites.iter().find(|s| s.child_ty == "Kid").expect("free-function birth");
    assert_eq!(free.child_decl, birth.child_decl);
    assert_eq!(free.child_key, birth.child_key);
}

/// A bound template and a specialized alias have the same accepting
/// key, while retaining the template's declaration for the model join.
#[test]
fn generic_births_retain_template_and_specialization() {
    let g = graph(r#"
        locus Cell<T> { }
        type IntCell = Cell<Int>;
        main locus App {
            accept(c: Cell<Int>) { }
            run() {
                let explicit: Cell<Int> = Cell { };
                let aliased = IntCell { };
            }
        }
        fn main() { App { }; }
    "#);
    let births: Vec<_> = g.sites.iter().filter(|s| s.enclosing_locus == "App").collect();
    assert_eq!(births.len(), 2);
    for birth in births {
        assert_eq!(birth.child_ty, "Cell_Int");
        assert_eq!(birth.child_key.as_deref(), Some("Cell_Int"));
        assert_eq!(g.declarations[birth.child_decl.unwrap()].name, "Cell");
        assert_eq!(birth.resolution, OwnerResolution::SelfOwned("App".into()));
    }
}

/// A qualified plain record and an unresolved path must not become
/// births of a user locus whose bare name happens to match their leaf.
#[test]
fn qualified_records_and_unknown_paths_are_not_locus_births() {
    let prog = parse_source(r#"
        locus Kid { }
        type ImportedRecord { n: Int = 0; }
        main locus App {
            run() { lib::Kid { }; unknown::Kid { }; }
        }
        fn main() { App { }; }
    "#).unwrap();
    let renames = vec![(vec!["lib".into(), "Kid".into()], "ImportedRecord".into())];
    let mut programs = BTreeMap::new();
    programs.insert(String::new(), &prog);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = renames;
    let (top, _) = build_top_scope(&bundle);
    let g = build_ownership_graph(&bundle, &top, &hale_types::placement::bundle_placement(&bundle, &top), &hale_types::entry::entry_row(&bundle));
    assert!(g.sites.is_empty(), "{:?}", g.sites);
    assert_eq!(g.free_fn_sites.len(), 1, "only the actual App birth remains");
}

/// The cross-pool law consumes the birth row's resolved key and value
/// context. An alias does not evade it, and bare aliases remain legal.
#[test]
fn aliased_cross_pool_birth_uses_the_resolved_plan() {
    for (body, count) in [("let s = Vessel { hull: 7 };", 1), ("Vessel { hull: 7 };", 0)] {
        let src = format!("type Vessel = Ship;\n{}", crosspool_src(body));
        let prog = parse_source(&src).unwrap();
        let errors: Vec<_> = hale_types::check_program(&prog).into_iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), count, "{body}: {errors:?}");
        for error in errors {
            assert!(error.message.starts_with(FIRE_AND_FORGET), "{error:?}");
            assert_eq!(&src[error.span.start.as_usize()..error.span.end.as_usize()], "Vessel { hull: 7 }");
        }
    }
}

#[test]
fn imported_cross_pool_birth_uses_the_resolved_plan() {
    for (body, count) in [("let s = lib::Vessel { hull: 7 };", 1), ("lib::Vessel { hull: 7 };", 0)] {
        let src = crosspool_src(body);
        let prog = parse_source(&src).unwrap();
        let renames = vec![(vec!["lib".into(), "Vessel".into()], "Ship".into())];
        let s = hale_frontend::snapshot::Snapshot::from_program(prog, renames, hale_frontend::snapshot::Config::check(false, false))
            .unwrap_or_else(|_| panic!("load"));
        let checked = s.demand_check().expect("check");
        let errors: Vec<_> = checked.diags.iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), count, "{body}: {errors:?}");
        for error in errors {
            assert!(error.message.contains("is fire-and-forget"), "{error:?}");
            assert_eq!(&src[error.span.start.as_usize()..error.span.end.as_usize()], "lib::Vessel { hull: 7 }");
        }
    }
}

/// Defaults belong to the fn's birth inventory too. Their declared
/// parameter type specializes a template, while the body keeps its
/// independent statement/value context.
#[test]
fn function_defaults_are_resolved_in_the_same_birth_walk() {
    let g = graph("locus Cell<T> { }\n\
        locus Kid { }\n\
        main locus App {\n\
            fn take(c: Cell<Int> = Cell { }) { Kid { }; let held = Kid { }; }\n\
        }\n\
        fn make(k: Kid = Kid { }) { }\n\
        fn main() { App { }; }\n");
    let cell = site(&g, "App", "Cell_Int");
    assert!(!cell.params_default && !cell.bare_statement);
    assert_eq!(cell.member.as_deref(), Some("take"));
    let kinds: Vec<_> = g.sites.iter().filter(|b| b.child_ty == "Kid").map(|b| b.bare_statement).collect();
    assert_eq!(kinds, [true, false]);
    assert!(g.free_fn_sites.iter().any(|b| b.child_key.as_deref() == Some("Kid")));
}

#[test]
fn birth_checks_contribute_value_births() {
    let g = graph("locus Kid { }\n\
        fn failed(k: Kid) -> Bool { return false; }\n\
        main locus App {\n\
            params { n: Int = 0; }\n\
            closure broken { captures: n; epoch inline; }\n\
            birth_check { failed(Kid { }) } -> violate broken;\n\
        }\n\
        fn main() { App { }; }\n");
    let birth = site(&g, "App", "Kid");
    assert!(!birth.bare_statement && !birth.params_default);
    assert_eq!(birth.child_key.as_deref(), Some("Kid"));
}

/// F.40 phase 3, C5: lowering derives no ownership graph of the user's
/// program. Its graph is the snapshot's rows, each found in the merged
/// program through the view's correspondence, followed by the stdlib's
/// rows, assembled as the snapshot's graph is: every user site resolves
/// as the snapshot's does, the stdlib's declarations follow the user's,
/// and the tower's accept relation (`accepts_ancestor`) is one relation
/// on both sides.
#[test]
fn lowerings_graph_is_the_snapshots_rows_through_the_correspondence() {
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
    let s = hale_frontend::snapshot::Snapshot::from_program(
        parse_source(src).unwrap(),
        Vec::new(),
        hale_frontend::snapshot::Config::check(false, false),
    )
    .unwrap_or_else(|_| panic!("load"));
    let snap = s.demand_ownership_graph().unwrap_or_else(|_| panic!("the graph is blocked"));
    let view = s.demand_lowering().unwrap_or_else(|_| panic!("the lowering view is blocked"));
    let low = &view.ownership;

    let user = |g: &OwnershipGraph| {
        g.sites
            .iter()
            .filter(|x| !x.enclosing_locus.starts_with("__Std"))
            .map(|x| (x.enclosing_locus.clone(), x.child_ty.clone(), x.resolution.clone(), x.span))
            .collect::<Vec<_>>()
    };
    assert_eq!(user(low), user(snap));
    assert_eq!(site(low, "B", "I").resolution, OwnerResolution::Ancestor("A".to_string()));
    assert_eq!(view.bubble.singleton.get(&("B".to_string(), "I".to_string())).map(String::as_str), Some("A"));

    // The stdlib's declarations follow the user's, which keep their order.
    let names = |g: &OwnershipGraph| g.declarations.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
    let (ours, theirs) = (names(snap), names(low));
    assert!(theirs.len() > ours.len() && theirs[..ours.len()] == ours[..], "{theirs:?}");
    assert!(theirs[ours.len()..].iter().all(|n| n.starts_with("__Std")), "{theirs:?}");

    for g in [snap, low] {
        assert!(g.rows.accepts_ancestor("A", "I") && !g.rows.accepts_ancestor("B", "I"));
    }

    // The view's scope is the snapshot's, the stdlib's declarations in it.
    let scope = s.demand_scope().unwrap_or_else(|_| panic!("the scope is blocked"));
    assert_eq!(format!("{:?}", view.top), format!("{scope:?}"));
    assert!(view.top.symbols.keys().any(|k| k.starts_with("__Std")));
}
