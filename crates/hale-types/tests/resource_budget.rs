//! The resource budget counts the placement table's resources (F.40
//! phase 3, P1 5 of 6; `notes/f40-placement-correspondence.md` § 2.8,
//! rows R-1 to R-7, and § 7's U-2 and U-3).
//!
//! Every case loads its seed the way `hale check --dump-resource-budget`
//! does — the frontend's load, the desugar sequence, the mint — from a
//! fixture under `fixtures/placement/`, and reads the budget over the
//! snapshot's table.

use std::path::PathBuf;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::resource_budget::{budget_for_programs, check_ceiling, ResourceBudget, ResourceCeiling};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/placement").join(name)
}

/// The budget of a fixture that checks clean.
fn budget(name: &str) -> ResourceBudget {
    let seed = fixture(name);
    let s = Snapshot::load(&seed, LoadMode::WholeSeed, &Disk, Config::check(seed.is_dir(), false))
        .unwrap_or_else(|_| panic!("{name} does not load"));
    let checked = s.demand_check().unwrap_or_else(|_| panic!("{name}: the check is blocked"));
    let errors: Vec<&str> = checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    assert!(errors.is_empty(), "{name} must check clean: {errors:?}");
    let table = s.demand_placement().unwrap_or_else(|_| panic!("{name}: placement is blocked"));
    let summary = s.demand_alloc_summary().unwrap_or_else(|_| panic!("{name}: no allocation summary"));
    budget_for_programs(&s.bundle(), table, summary)
}

/// R-1: `pinned(replicas = 3)` is three threads, one per replica, beside
/// one more pinned field (`q`); the nested subscribers inherit their
/// anchors' threads and add none.
#[test]
fn replicated_pinned_counts_k_threads() {
    let b = budget("nested_inheritance.hl");
    assert_eq!(b.anchored_threads, Ok(4), "`q` and `r`'s three replicas");
    assert_eq!(b.adapter_threads, 0);
    assert_eq!(b.threads(), Ok(4));
}

/// R-2: three instances on one pool are one worker, not three.
#[test]
fn three_instances_on_one_pool_are_one_worker() {
    let b = budget("budget_pools.hl");
    assert_eq!(b.cooperative_pools.iter().map(String::as_str).collect::<Vec<_>>(), ["io", "ws"]);
}

/// R-3: an affinity is a column of its pool, never a thread: two pools
/// with affinities add no OS thread.
#[test]
fn pool_affinity_adds_no_thread() {
    let b = budget("budget_pools.hl");
    assert_eq!(b.threads(), Ok(0));
    assert_eq!(b.cooperative_pools.len(), 2);
}

/// R-4: only the deployed root's rows count. An imported `__lib_` root's
/// pinned entry costs nothing, whether or not the seed has a root of its
/// own; the seed's own root's does.
#[test]
fn an_imported_roots_entries_cost_nothing() {
    let alone = budget("imported/no_own_main");
    assert_eq!(alone.threads(), Ok(0), "the imported root is never deployed");
    let own = budget("imported/own_main");
    assert_eq!(own.threads(), Ok(1), "the seed's own pinned field, and not the import's");
}

/// R-5: a root built by a fn called in a loop has no static bound, so its
/// pinned field's threads are an uncertainty that carries the reason, and
/// a declared thread ceiling fails with it.
#[test]
fn a_root_built_in_a_called_loop_is_uncertain() {
    let b = budget("budget_called_loop.hl");
    assert_eq!(b.threads(), Err("built in `boot`, which is called in a loop".to_string()));
    let over = check_ceiling(&b, &ResourceCeiling { pinned_threads: Some(64), ..Default::default() });
    assert_eq!(
        over,
        ["pinned_threads (OS threads): uncertain (built in `boot`, which is called in a loop), so no count is \
          within the declared ceiling of 64"]
    );
    assert!(check_ceiling(&b, &ResourceCeiling::default()).is_empty(), "no thread ceiling, nothing to fail");
    assert!(
        b.render().contains("OS threads (placement):   uncertain (built in `boot`, which is called in a loop)\n"),
        "{}",
        b.render()
    );
}

/// R-6, the partition by construction scope: one adapter in
/// `bindings { }` and one root field placed `pinned` are two threads, not
/// three (the adapter's pinned domain is counted once, as an adapter).
#[test]
fn one_adapter_and_one_pinned_child_are_two_threads() {
    let b = budget("adapter.hl");
    assert_eq!((b.anchored_threads.clone(), b.adapter_threads), (Ok(1), 1));
    assert_eq!(b.threads(), Ok(2));
}

/// R-5 and R-6: two constructions of the root, one `AtMost(3)` and one
/// `Once`, each with one pinned field, and one prelude adapter: 3 + 1 + 1,
/// never (3 + 1) × 2. The adapter is built once, whatever the root's
/// bound.
#[test]
fn an_adapter_counts_once_under_several_root_constructions() {
    let b = budget("budget_adapter_bounds.hl");
    assert_eq!(b.anchored_threads, Ok(4), "three live occurrences of one construction and one of the other");
    assert_eq!(b.adapter_threads, 1);
    assert_eq!(b.threads(), Ok(5));
}

/// R-7 (U-2): `pool = main` is the program's main thread, never a worker
/// pool; the dump shows main on a line of its own whether or not a
/// program spells it.
#[test]
fn the_main_pool_is_not_a_worker() {
    let b = budget("budget_pools.hl");
    assert!(!b.cooperative_pools.contains("main"), "{:?}", b.cooperative_pools);
    let dump = b.render();
    assert!(dump.contains("main thread:               1 (the program's own; not a pool)\n"), "{dump}");
    assert!(dump.contains("cooperative pools:         2  [io, ws]\n"), "{dump}");
}

/// The dump says what it counts and what it omits (U-3): the placement
/// threads in two parts, and the threads the runtime spawns outside
/// placement on a "not counted" line, never folded into the total.
#[test]
fn the_dump_says_what_it_counts_and_what_it_omits() {
    let dump = budget("adapter.hl").render();
    for line in [
        "# OS threads are the placement table's: one per pinned anchor (a root field\n",
        "# placed `pinned`, one per replica) for each live occurrence of the construction\n",
        "# that builds it, and one per adapter in the root's `bindings { }`, built once.\n",
        "# A cooperative pool is one worker however many instances run on it; the main\n",
        "# thread is the program's own, never a pool. Not counted: a transport\n",
        "# binding's reader thread, and the serve thread a stdlib transport's birth spawns.\n",
        "OS threads (placement):   2\n",
        "    pinned anchors:        1\n",
        "    adapters:              1\n",
        "not counted:               the reader threads of 0 transport binding(s), and any serve thread a stdlib \
         transport's birth spawns\n",
    ] {
        assert!(dump.contains(line), "missing {line:?} in:\n{dump}");
    }
}
