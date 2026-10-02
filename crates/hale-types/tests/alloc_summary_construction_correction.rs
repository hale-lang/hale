//! Classified corrections to how the one allocation summary is built,
//! pinned (F.40 phase 3, E3a part B).
//!
//! The snapshot's summary (`Snapshot::demand_alloc_summary`) holds the
//! checked programs with the stdlib's analysis copy beside them. When
//! these corrections landed its readers were the check's effects
//! certificate engine and the effect rows; the dump, the advisory, the
//! model, the budgets and the frontier moved onto it after them. Each
//! correction is pinned here per target, on the whole summary (its dump
//! with the copy's rows, its leak sites) and on what those readers
//! answer, against the old answer reproduced from the corrected summary,
//! and alone: the later corrections' fields are cleared on both sides of
//! an earlier one's pin.
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
//!
//! **The unbounded-invocation fixpoint seeds from what the program
//! reaches.** Every fn of the summary used to seed it, so every loop of
//! the stdlib's analysis copy counted whether or not the program ever
//! ran it: `__http_handle_one_conn`'s and `__http_run_chain`'s loops
//! reach any user `handle` through the interface fan-out, and made it
//! invoked unboundedly in programs that never start the HTTP loop. Now
//! only the fns the program reaches (`AllocSummary::reached`: its own,
//! what their calls reach, and the hooks and handlers of the loci they
//! start) seed it or call. Six targets' own fns move, two leak sites go,
//! and a program that starts the loop (`dna/api`, `dna/ui`) or runs the
//! router's chain (`69-http-router`, `router_middleware_test`) keeps its
//! facts; every other target's change is the stdlib copy's own fns
//! alone (no longer invoked unboundedly by loops nothing starts). No
//! effect row, manifest row or certificate reads the fixpoint. The old
//! answer is the corrected summary with `reached` cleared.
//!
//! **The run-to-exit rule reads the program's own entries.** A program
//! with a `main` and no long-lived entry (`run`, a bus handler) has no
//! leak sites. The rule read every fn of the summary, and the stdlib's
//! analysis copy always carries `run` hooks, so it never applied. Now it
//! reads the program's own fns only (`AllocSummary::analysis_copy` names
//! the copy's). 77 targets are run-to-exit programs: each loses the 21
//! leak sites of the copy's own fns, and two lose two of `main`'s own.
//! No site is added and no other target moves. The old answer is the
//! corrected summary with `analysis_copy` cleared.
//!
//! **A program's declaration is the row where the copy declares the
//! same name.** The summary keys a fn by its name, and the copy's body
//! came last, so where a checked program declares a name the copy
//! declares, the copy's row took the key and the program had none of
//! its own: `hale check` over a stdlib source file saw the copy's bodies
//! (the copy's spans, resolved in the copy's scope), and since commit 6
//! its dump and advisory reported none of the file's fns. Now the copy's
//! declaration of a name the program declares (a free fn, a locus, an
//! interface) stays out. Only stdlib source shares such a name: no
//! target, and of the corpus programs the 27 stdlib files. Their checks
//! get their own rows back: against the commit before, the dump moves
//! in all 27 and the advisory's warnings return in seven (`api`,
//! `http_client`, `lang`, `name`, `tagged`, `text`, `yaml`), as part A
//! reported them; and `http_client.hl`'s `__http_client_header` loses
//! its `alloc` (its call to `__http_find_header_in_block`, declared by
//! another stdlib file, is unresolved in the program's own scope), so
//! that check's effect manifest, model and `shape_hash` move
//! (`fccb52d86a539447` → `371edb8609756fca`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::alloc_summary::{summarize_identified, AllocSummary, Callee, EntryKind, FnKey, LeakSite};
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
        // This correction alone: the later ones cleared on both sides,
        // and the whole summary's rows rendered.
        let mut now = now.clone();
        now.reached = None;
        now.analysis_copy.clear();
        now.analysis_copy_loci.clear();
        let now = &now;
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

/// The dump lines of the program's own fns that differ, old then now:
/// a fn's block runs from its `fn` line to the next.
fn own_changed_lines(old: &str, now: &str, stdlib: &BTreeSet<String>) -> Vec<String> {
    assert_eq!(old.lines().count(), now.lines().count(), "the dump keeps its shape");
    let mut own = false;
    let mut out = Vec::new();
    for (o, n) in old.lines().zip(now.lines()) {
        if let Some(rest) = o.strip_prefix("fn ") {
            own = !stdlib.contains(rest.split_whitespace().next().unwrap_or(""));
        }
        if own && o != n {
            out.push(format!("- {}", o.trim()));
            out.push(format!("+ {}", n.trim()));
        }
    }
    out
}

/// What the reached correction moves in one target: the program's own
/// fns no longer invoked unboundedly, the leak sites that go, how many
/// fns the fixpoint holds before and after, and the program's own dump
/// lines that change. Panics if it invokes a fn or adds a leak site, or
/// leaves one of the program's own fns unreached.
#[derive(Debug, Default)]
struct Unreached {
    own: Vec<String>,
    leaks: Vec<String>,
    invoked: (usize, usize),
    lines: Vec<String>,
}

fn unreached(target: &str, stdlib: &BTreeSet<FnKey>) -> Option<Unreached> {
    let names: BTreeSet<String> = stdlib.iter().map(FnKey::display).collect();
    over_target(target, |_, now| {
        // This correction alone: the later one cleared on both sides,
        // and the whole summary's rows rendered.
        let mut now = now.clone();
        now.analysis_copy.clear();
        now.analysis_copy_loci.clear();
        let now = &now;
        let mut old = now.clone();
        old.reached = None;
        let reached = now.reached.as_ref().expect("the summary holds the stdlib's copy");
        for k in now.fns.keys().filter(|k| !stdlib.contains(k)) {
            assert!(reached.contains(k), "{target}: {} is the program's own and unreached", k.display());
        }
        let (was, is) = (old.unbounded_invoked(), now.unbounded_invoked());
        assert!(is.is_subset(&was), "{target}: the correction invoked a fn unboundedly");
        let leaks = |s: &AllocSummary| -> BTreeSet<String> {
            s.leak_sites()
                .iter()
                .map(|l| format!("{} {:?} @{}..{} {:?}", l.owner.display(), l.kind, l.span.start.0, l.span.end.0, l.reason))
                .collect()
        };
        let (lw, li) = (leaks(&old), leaks(now));
        assert!(li.is_subset(&lw), "{target}: the correction added a leak site");
        Unreached {
            own: was.difference(&is).filter(|k| !stdlib.contains(k)).map(FnKey::display).collect(),
            leaks: lw.difference(&li).cloned().collect(),
            invoked: (was.len(), is.len()),
            lines: own_changed_lines(&old.render(), &now.render(), &names),
        }
    })
}

/// The six targets pinned below are the only ones whose own fns or leak
/// sites the reached correction moves.
#[test]
fn reached_no_other_target_changes() {
    let stdlib = stdlib_fns();
    let mut corrected = Vec::new();
    for t in targets() {
        let Some(u) = unreached(&t, &stdlib) else { continue };
        if !u.own.is_empty() || !u.leaks.is_empty() || !u.lines.is_empty() {
            corrected.push(t);
        }
    }
    assert_eq!(
        corrected,
        [
            "crates/hale-codegen/tests/fixtures/examples/60-perspective-slot",
            "crates/hale-codegen/tests/fixtures/examples/65-perspective-ctor-override",
            "crates/hale-codegen/tests/fixtures/examples/90-unowned-literal-positions",
            "dna/oidc",
            "tests/hale/is_route_test.hl",
            "tests/hale/perspective_ctor_override_test.hl",
        ]
    );
}

fn pinned_unreached(target: &str, invoked: (usize, usize), own: &[&str], leaks: &[&str], lines: &[&str]) {
    let u = unreached(target, &stdlib_fns()).expect("the target loads");
    let strs = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
    assert_eq!(
        (u.own, u.leaks, u.lines, u.invoked),
        (strs(own), strs(leaks), strs(lines), invoked),
        "{target}: what the reached correction moves is pinned; a difference is classified before the pin moves"
    );
}

/// The two perspective fixtures and their `tests/hale` twin implement the
/// HTTP handler interface and never start the HTTP loop.
#[test]
fn reached_perspective_examples() {
    let lines = [
        "- fn Gateway::handle   [invoked-unboundedly]",
        "+ fn Gateway::handle",
        "- fn RouterV1::route   [invoked-unboundedly]",
        "+ fn RouterV1::route",
    ];
    for t in [
        "crates/hale-codegen/tests/fixtures/examples/60-perspective-slot",
        "crates/hale-codegen/tests/fixtures/examples/65-perspective-ctor-override",
        "tests/hale/perspective_ctor_override_test.hl",
    ] {
        pinned_unreached(t, (115, 0), &["Gateway::handle", "RouterV1::route"], &[], &lines);
    }
}

/// 90-unowned-literal-positions declares its own `Server` with a
/// `handle`, and never starts the stdlib's.
#[test]
fn reached_unowned_literal_positions() {
    pinned_unreached(
        "crates/hale-codegen/tests/fixtures/examples/90-unowned-literal-positions",
        (115, 0),
        &["Provider::submit", "Server::handle"],
        &["Provider::submit CollectionInsert(\"vec\") @1292..1320 InvokedUnboundedly"],
        &[
            "- fn Provider::submit   [invoked-unboundedly]",
            "+ fn Provider::submit",
            "- alloc vec-insert       escaping=self-store  ACCUMULATES-UNBOUNDED  reclaim@locus-dissolve @1292..1320  <-- LEAK",
            "+ alloc vec-insert       escaping=self-store  once-per-invocation    reclaim@locus-dissolve @1292..1320",
            "- fn Server::handle   [invoked-unboundedly]",
            "+ fn Server::handle",
        ],
    );
}

/// dna/oidc is the issuer's library seed: `oidc/serve` starts the HTTP
/// loop over its `StubIssuer` in a program of its own.
#[test]
fn reached_oidc() {
    pinned_unreached(
        "dna/oidc",
        (137, 2),
        &[
            "b64",
            "decode",
            "form_field",
            "signed_id_token",
            "StubIssuer::handle",
            "StubIssuer::json",
            "StubIssuer::service_token",
            "StubIssuer::token_answer",
        ],
        &[],
        &[
            "- fn b64   [invoked-unboundedly]",
            "+ fn b64",
            "- fn decode   [invoked-unboundedly]",
            "+ fn decode",
            "- fn form_field   [invoked-unboundedly]",
            "+ fn form_field",
            "- fn signed_id_token   [invoked-unboundedly]",
            "+ fn signed_id_token",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @5079..5128",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @5079..5128",
            "- fn StubIssuer::handle   [invoked-unboundedly]",
            "+ fn StubIssuer::handle",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @7454..7693",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @7454..7693",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @7529..7691",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @7529..7691",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @8394..8471",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @8394..8471",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @8701..8787",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @8701..8787",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @9063..9208",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @9063..9208",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @9135..9196",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @9135..9196",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @9908..10017",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @9908..10017",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @10297..10407",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @10297..10407",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @11144..11252",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @11144..11252",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @11279..11333",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @11279..11333",
            "- fn StubIssuer::json   [invoked-unboundedly]",
            "+ fn StubIssuer::json",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @6360..6447",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @6360..6447",
            "- fn StubIssuer::service_token   [invoked-unboundedly]",
            "+ fn StubIssuer::service_token",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @12119..12228",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @12119..12228",
            "- fn StubIssuer::token_answer   [invoked-unboundedly]",
            "+ fn StubIssuer::token_answer",
        ],
    );
}

/// is_route_test calls its `Api` directly and never starts the loop.
#[test]
fn reached_is_route_test() {
    pinned_unreached(
        "tests/hale/is_route_test.hl",
        (117, 0),
        &["Api::handle", "Api::list", "Api::rename", "Api::show"],
        &["Api::handle StructLit(\"std::http::Response\") @1449..1496 InvokedUnboundedly"],
        &[
            "- fn Api::handle   [invoked-unboundedly]",
            "+ fn Api::handle",
            "- alloc struct std::http::Response escaping=return      ACCUMULATES-UNBOUNDED  reclaim@locus-dissolve @1449..1496  <-- LEAK",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @1449..1496",
            "- fn Api::list   [invoked-unboundedly]",
            "+ fn Api::list",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @1677..1731",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @1677..1731",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @1718..1729",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @1718..1729",
            "- fn Api::rename   [invoked-unboundedly]",
            "+ fn Api::rename",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @2146..2224",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @2146..2224",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @2187..2222",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @2187..2222",
            "- fn Api::show   [invoked-unboundedly]",
            "+ fn Api::show",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @1910..1965",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @1910..1965",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @1951..1963",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @1951..1963",
        ],
    );
}

/// What the run-to-exit correction removes in one target, as the leak
/// sites of the copy's fns (counted) and of the program's own (listed);
/// `None` when it moves nothing. Panics if it adds a site, or moves one
/// in a program with a long-lived entry of its own or no `main`.
fn run_to_exit(target: &str) -> Option<(usize, Vec<String>)> {
    over_target(target, |_, now| {
        let mut old = now.clone();
        old.analysis_copy.clear();
        let site = |l: &LeakSite| {
            format!("{} {:?} @{}..{} {:?}", l.owner.display(), l.kind, l.span.start.0, l.span.end.0, l.reason)
        };
        let is: BTreeSet<String> = now.leak_sites().iter().map(site).collect();
        let gone: Vec<LeakSite> = old.leak_sites().into_iter().filter(|l| !is.contains(&site(l))).collect();
        assert!(is.len() + gone.len() == old.leak_sites().len(), "{target}: the correction added a leak site");
        if gone.is_empty() {
            return None;
        }
        let own_entry = |e: EntryKind| now.fns.values().any(|f| now.is_own(&f.key) && f.entry == Some(e));
        assert!(
            own_entry(EntryKind::Main) && !own_entry(EntryKind::Run) && !own_entry(EntryKind::BusHandler),
            "{target}: a site moved in a program that is not run-to-exit"
        );
        assert!(is.is_empty(), "{target}: a run-to-exit program keeps a leak site");
        let copy = gone.iter().filter(|l| !now.is_own(&l.owner)).count();
        let own = gone.iter().filter(|l| now.is_own(&l.owner)).map(site).collect();
        Some((copy, own))
    })
    .flatten()
}

/// Every run-to-exit program loses the copy's 21 sites; two also lose
/// their own `main`'s in-loop sites; no other target moves.
#[test]
fn own_entries_run_to_exit_programs() {
    let mut moved = 0;
    let mut own = Vec::new();
    for t in targets() {
        let Some((copy, mine)) = run_to_exit(&t) else { continue };
        moved += 1;
        assert_eq!(copy, 21, "{t}: the copy's leak sites");
        own.extend(mine.into_iter().map(|s| format!("{t}: {s}")));
    }
    assert_eq!(moved, 77, "the run-to-exit programs among the targets");
    assert_eq!(
        own,
        [
            "tests/hale/api_context_test.hl: main StringConcat @12358..12422 InUnboundedLoop",
            "tests/hale/api_context_test.hl: main StringConcat @12358..12393 InUnboundedLoop",
            "tests/hale/chains_tranche2_test.hl: main CollectionInsert(\"vec\") @1882..1910 InUnboundedLoop",
            "tests/hale/chains_tranche2_test.hl: main CollectionInsert(\"vec\") @2095..2120 InUnboundedLoop",
        ]
    );
}

/// A program that starts the HTTP loop, or runs the router's chain,
/// keeps its handlers invoked unboundedly.
#[test]
fn reached_a_started_loop_keeps_its_facts() {
    for (t, kept) in [
        ("dna/api", &["Api::handle"][..]),
        ("dna/ui", &["Ui::handle"][..]),
        ("crates/hale-codegen/tests/fixtures/examples/69-http-router", &["Count::handle", "Hello::handle", "Stamp::after", "Stamp::before"][..]),
        ("tests/hale/router_middleware_test.hl", &["Hello::handle", "Stamp::after", "Stamp::before"][..]),
    ] {
        over_target(t, |_, now| {
            let is = now.unbounded_invoked();
            for k in kept {
                assert!(is.iter().any(|f| f.display() == *k), "{t}: {k} is no longer invoked unboundedly");
            }
        })
        .expect("the target loads");
    }
}

/// The top-level names (free fns, loci, interfaces) a program declares.
fn top_level_names(program: &hale_syntax::ast::Program) -> BTreeSet<String> {
    use hale_syntax::ast::TopDecl;
    program
        .items
        .iter()
        .filter_map(|item| match item {
            TopDecl::Fn(f) => Some(f.name.name.clone()),
            TopDecl::Locus(l) => Some(l.name.name.clone()),
            TopDecl::Interface(i) => Some(i.name.name.clone()),
            _ => None,
        })
        .collect()
}

/// The stdlib's source files, by their path from the root.
fn stdlib_sources() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root().join("crates/hale-stdlib/hl"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "hl"))
        .map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().to_string())
        .collect();
    out.sort();
    out
}

/// Only stdlib source shares a top-level name with the analysis copy:
/// no target, and no corpus program outside `crates/hale-stdlib/hl`, so
/// the correction moves nothing else.
#[test]
fn program_declaration_only_stdlib_source_shares_a_name() {
    let copy = top_level_names(hale_types::stdlib_bodies::program().expect("the stdlib parses"));
    for t in targets() {
        let shared = over_target(&t, |snap, _| {
            let bundle = snap.bundle();
            bundle.programs.values().flat_map(|p| top_level_names(p)).filter(|n| copy.contains(n)).collect::<Vec<_>>()
        });
        assert_eq!(shared.unwrap_or_default(), Vec::<String>::new(), "{t} shares a name with the copy");
    }
    let mut sharing = 0;
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        if top_level_names(&program).iter().any(|n| copy.contains(n)) {
            assert!(p.origin.starts_with("crates/hale-stdlib/hl/"), "{} shares a name with the copy", p.origin);
            sharing += 1;
        }
    }
    assert_eq!(sharing, 27, "the stdlib source files that share a name with the copy");
}

/// A stdlib file checked as itself keeps its own rows: every fn of the
/// program alone is the program's in the summary, with the program's
/// body (its sites and loops where the program alone has them). The
/// copy's row of the same name used to take the key, and the file then
/// had none of its shared fns as its own.
#[test]
fn program_declaration_stdlib_files_checked_as_themselves() {
    let mut keys = 0;
    for t in stdlib_sources() {
        let Some(n) = over_target(&t, |snap, now| {
            let bundle = snap.bundle();
            let alone: Vec<_> = bundle.programs.values().map(|p| (*p, &bundle.snapshot)).collect();
            let alone = summarize_identified(&alone, &bundle.import_renames);
            let spans = |f: &hale_types::alloc_summary::FnSummary| {
                let sites: Vec<_> = f.sites.iter().map(|s| (s.span.start.0, s.span.end.0)).collect();
                let loops: Vec<_> = f.loops.iter().map(|l| (l.span.start.0, l.span.end.0)).collect();
                (sites, loops)
            };
            for (k, f) in &alone.fns {
                let row = now.fns.get(k).unwrap_or_else(|| panic!("{t}: {} has no row", k.display()));
                assert!(now.is_own(k), "{t}: {} is the copy's", k.display());
                assert_eq!(spans(row), spans(f), "{t}: {} is not the program's body", k.display());
            }
            alone.fns.len()
        }) else {
            continue;
        };
        keys += n;
    }
    assert!(keys > 0, "the stdlib files have rows of their own");
}

/// `http_client.hl` checked as itself: `__http_client_header` calls
/// `__http_find_header_in_block`, which another stdlib file declares, so
/// in the program's own scope the call is unresolved and the row carries
/// no `alloc`; the copy's same-named body resolved it in the copy's
/// scope, and its `{alloc}` reached the effect rows, the model's effect
/// column and `shape_hash`.
#[test]
fn program_declaration_http_client_header() {
    let manifest = over_target("crates/hale-stdlib/hl/http_client.hl", |snap, _| {
        let rows = snap.demand_effects().expect("the effect rows");
        hale_types::dump_effects_manifest(&snap.bundle(), rows)
    })
    .expect("http_client.hl has a summary");
    assert!(manifest.contains("__http_check_scheme  does={alloc}"), "the manifest lists a fn's classes:\n{manifest}");
    assert!(!manifest.contains("__http_client_header  does="), "the row carries no class:\n{manifest}");
}
