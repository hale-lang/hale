//! Classified corrections to how the one allocation summary is built,
//! pinned (F.40 phase 3, E3a part B).
//!
//! The snapshot's summary (`Snapshot::demand_alloc_summary`) holds the
//! checked programs with the stdlib's analysis copy beside them. Its
//! readers today are the check's effects certificate engine and the
//! effect rows; the dump, the advisory, the model, the budgets and the
//! frontier move onto it after these corrections. Each correction is
//! pinned here per target, on what those readers answer, against the old
//! answer reproduced from the corrected summary.
//!
//! **Each seed's names resolve in its own scope.** The summary used to
//! resolve a bare free-fn name against every program it held, so a
//! stdlib body's builtin `count(...)` (`http.hl`'s `__http_path_param`,
//! `metrics.hl`'s histogram) named a user fn that happened to be called
//! `count`, which then read as called from the stdlib, invoked
//! unboundedly through its loops. Of the 170 targets `hale check` is run
//! over here (the corpus fixtures, the `tests/hale` programs and the DNA
//! seeds), `let_block_scope_test` is the only one with such a name: its
//! four stdlib callers' five edges no longer resolve to its `count`, no
//! effect class moves, and it holds no certificate. The old answer is
//! the corrected summary with each stdlib body's bare call re-resolved
//! against every program, as the shared scope did.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::alloc_summary::{summarize_identified, AllocSummary, Callee, FnKey};
use hale_types::effect_rows::derive_effect_rows;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Every target `hale check` is run over here: the corpus fixtures, the
/// `tests/hale` programs and the DNA seeds, by their path from the root.
fn targets() -> Vec<String> {
    let root = root();
    let mut out = Vec::new();
    let mut push_dir = |dir: &str, keep: &dyn Fn(&Path) -> bool| {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if keep(&p) {
                out.push(p.strip_prefix(&root).unwrap().to_string_lossy().to_string());
            }
        }
    };
    push_dir("crates/hale-codegen/tests/fixtures/examples", &|p| p.is_dir());
    push_dir("tests/hale", &|p| p.to_string_lossy().ends_with("_test.hl"));
    push_dir("dna", &|p| {
        p.is_dir()
            && !p.ends_with("tests")
            && std::fs::read_dir(p).unwrap().any(|f| f.unwrap().path().extension().is_some_and(|x| x == "hl"))
    });
    out.sort();
    out
}

/// Run `f` over the target's snapshot, loaded as `hale check <target>`
/// loads it, and its summary. On a thread of its own: a whole DNA seed's
/// walk is deep. `None` when the target does not load or its scope is
/// blocked (it has no summary).
fn over_target<T: Send>(target: &str, f: impl FnOnce(&Snapshot, &AllocSummary) -> T + Send) -> Option<T> {
    let path = root().join(target);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config).ok()?;
                let summary = snap.demand_alloc_summary().ok()?;
                Some(f(&snap, summary))
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The fns of the stdlib's analysis copy.
fn stdlib_fns() -> BTreeSet<FnKey> {
    let program = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    let ids = hale_types::stdlib_bodies::identities().expect("the stdlib is minted");
    summarize_identified(&[(program, ids)], &[]).fns.into_keys().collect()
}

/// The shared scope's answer: `now` with every bare free-fn call of a
/// stdlib body that names a fn of the checked programs resolved to it.
/// A bare call is the edge with no receiver and no receiver type whose
/// name is not a path. Returns the summary and the edges it resolved, as
/// `caller -> callee`.
fn shared_scope(now: &AllocSummary, stdlib: &BTreeSet<FnKey>) -> (AllocSummary, Vec<String>) {
    let mut old = now.clone();
    let mut resolved = Vec::new();
    for (key, f) in old.fns.iter_mut() {
        if !stdlib.contains(key) {
            continue;
        }
        for c in &mut f.calls {
            let Callee::Unresolved(name) = &c.callee else { continue };
            let callee = FnKey::free_fn(name.clone());
            if c.recv_ty.is_none()
                && !c.receiver_present
                && !name.contains("::")
                && now.fns.contains_key(&callee)
                && !stdlib.contains(&callee)
            {
                resolved.push(format!("{} -> {}", key.display(), callee.display()));
                c.callee = Callee::Resolved(callee);
            }
        }
    }
    (old, resolved)
}

/// The dump lines that differ, old then now, in order.
fn changed_lines(old: &str, now: &str) -> Vec<String> {
    assert_eq!(old.lines().count(), now.lines().count(), "the dump keeps its shape");
    old.lines()
        .zip(now.lines())
        .filter(|(o, n)| o != n)
        .flat_map(|(o, n)| [format!("- {}", o.trim()), format!("+ {}", n.trim())])
        .collect()
}

/// `let_block_scope_test` is the only target with a stdlib body whose
/// bare name names one of its fns.
#[test]
fn own_scope_no_other_target_changes() {
    let stdlib = stdlib_fns();
    let mut corrected = Vec::new();
    for t in targets() {
        let edges = over_target(&t, |_, now| shared_scope(now, &stdlib).1).unwrap_or_default();
        if !edges.is_empty() {
            corrected.push(t);
        }
    }
    assert_eq!(corrected, ["tests/hale/let_block_scope_test.hl"]);
}

/// let_block_scope_test: five edges of four stdlib fns; the user
/// `count` is no longer invoked unboundedly; no effect class moves; no
/// certificate to move.
#[test]
fn own_scope_let_block_scope_test() {
    let stdlib = stdlib_fns();
    over_target("tests/hale/let_block_scope_test.hl", |snap, now| {
        let (old, edges) = shared_scope(now, &stdlib);
        assert_eq!(
            edges,
            [
                "__http_path_param -> count",
                "__metrics_count_out_of_order -> count",
                "__metrics_render_histogram -> count",
                "__metrics_render_histogram -> count",
                "__StdMetricsHistogram::observe -> count",
            ]
        );
        assert_eq!(
            changed_lines(&old.render(), &now.render()),
            [
                "- # 438 fns, 34 entry points, 114 invoked-unboundedly",
                "+ # 438 fns, 34 entry points, 113 invoked-unboundedly",
                "- call  count loop_depth=0 result=local",
                "+ call  <unresolved: count> loop_depth=0 result=local",
                "- call  count loop_depth=0 result=local",
                "+ call  <unresolved: count> loop_depth=0 result=local",
                "- call  count loop_depth=0 result=local",
                "+ call  <unresolved: count> loop_depth=0 result=local",
                "- call  count loop_depth=0 result=local",
                "+ call  <unresolved: count> loop_depth=0 result=local",
                "- fn count   [invoked-unboundedly]",
                "+ fn count",
                "- call  count loop_depth=0 result=local",
                "+ call  <unresolved: count> loop_depth=0 result=local",
            ]
        );
        // The effect rows over each: the four callers' targets move, no
        // fn's effects do.
        let bundle = snap.bundle();
        let top = snap.demand_scope().expect("the scope");
        let rows_now = snap.demand_effects().expect("the effect rows");
        let rows_old = derive_effect_rows(&bundle, top, Arc::new(old));
        let moved: Vec<String> = rows_now
            .rows
            .iter()
            .filter(|(k, r)| rows_old.rows.get(*k).is_none_or(|o| o.targets != r.targets))
            .map(|(k, _)| k.display())
            .collect();
        assert_eq!(
            moved,
            [
                "__http_path_param",
                "__metrics_count_out_of_order",
                "__metrics_render_histogram",
                "__StdMetricsHistogram::observe",
            ]
        );
        for (k, r) in &rows_now.rows {
            let o = &rows_old.rows[k];
            assert_eq!((o.effects, o.unknown), (r.effects, r.unknown), "{}: an effect moved", k.display());
        }
        assert!(snap.demand_effect_certificates().expect("the certificates").is_empty());
    })
    .expect("let_block_scope_test loads");
}
