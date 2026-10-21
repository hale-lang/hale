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
//! corrected summary with `analysis_copy` cleared. (Since GH #1490 the
//! copy's 21 are no leak sites to begin with — each is a value a copy fn
//! builds in its own loop and returns, judged once per call — so only
//! the programs that lose a site of their own still move.)
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
//!
//! **The summary collects module-nested bodies** (E3a part C). Every
//! declaration pass walked a program's top level only, so a fn or a
//! locus declared inside `module { … }` had no row: a call into one was
//! the unresolved bare name, which every effect walk read as nothing,
//! and the model listed the declaration with an `UnanalyzedBody` hole
//! (a module-nested locus with members `analyzable: false`). A module is
//! a namespace, not an analysis boundary (GH #764), and the passes walk
//! it now. Of the targets only `91-module-decls` has module-nested
//! bodies: it gains seven rows, `App::run`'s seven calls into them
//! resolve, no effect class moves and it holds no certificate. Across
//! the corpus every program with a module-nested body moves its dump,
//! its effect rows and its model, `shape_hash` included (the model's
//! holes close and its summarized universe grows); `91-module-decls`
//! goes `e433cc0599156f87` → `86745b4e2324e4a2`. Where a nested body
//! does something its caller's effect row now says so: `App::run`
//! calling a module-nested `danger` that runs a process was
//! `{publish, alloc}` with nothing unknown, a false proof of absence.
//! A `causes:` law through a module-nested fn, which could not be
//! certified, holds. The old answer is the corrected summary with the
//! module-nested rows removed and each call into one unresolved by its
//! bare name (measured against the base build's dump and effect rows on
//! every corpus program with a module-nested body: equal).

#[path = "support/entries.rs"]
mod entries;
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
            && hale_corpus::holds_hl(p)
    });
    // the host's verb program, a seed inside the host's (GH #1490)
    out.push("dna/host/verb".to_string());
    out.sort();
    out
}

/// Run `f` over the target's snapshot, loaded as `hale check <target>`
/// loads it, and its summary with the later correction's startable loci
/// (GH #1490) cleared, so each earlier correction is pinned alone. On a
/// thread of its own: a whole DNA seed's walk is deep. `None` when the
/// target does not load or its scope is blocked (it has no summary).
fn over_target<T: Send>(target: &str, f: impl FnOnce(&Snapshot, &AllocSummary) -> T + Send) -> Option<T> {
    over_target_as_built(target, |snap, now| {
        let mut now = now.clone();
        now.startable = None;
        f(snap, &now)
    })
}

/// The effect rows over the summary beside the whole stdlib copy
/// ([`over_target_as_built`]'s).
fn whole_effects(snap: &Snapshot) -> hale_types::effect_rows::EffectRows {
    let views = snap.demand_owned_views().expect("the owned-view column");
    let summary = hale_types::alloc_summary::derive_alloc_summary_over(&snap.bundle(), views);
    derive_effect_rows(&snap.bundle(), snap.demand_scope().expect("the scope"), Arc::new(summary))
}

/// [`over_target`] with the summary beside the whole stdlib copy, as the
/// snapshot's walk and owned-view column give it: the corrections pinned
/// here are of the copy's rows too, which the check's summary holds only
/// where the program reaches them.
fn over_target_as_built<T: Send>(target: &str, f: impl FnOnce(&Snapshot, &AllocSummary) -> T + Send) -> Option<T> {
    let path = root().join(target);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config).ok()?;
                snap.demand_alloc_summary().ok()?;
                let views = snap.demand_owned_views().ok()?;
                let summary = hale_types::alloc_summary::derive_alloc_summary_over(&snap.bundle(), views);
                Some(f(&snap, &summary))
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// `summary` with the reclaim boundary's correction (E3b) cleared: no
/// fn's frame classified, and no site reclaimed at its fn's return.
fn without_frames(summary: &AllocSummary) -> AllocSummary {
    let mut s = summary.clone();
    for f in s.fns.values_mut() {
        f.frame = None;
        for site in &mut f.sites {
            if site.reclaim == hale_types::alloc_summary::ReclaimScope::FnReturn {
                site.reclaim = hale_types::alloc_summary::ReclaimScope::EnclosingLocus;
            }
        }
    }
    s
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
            let Some(callee) = now.resolve(None, name).cloned() else { continue };
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
        let mut now = without_frames(now);
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
            // the whole summary's header counts the copy's fns: 435 since
            // the HTTP server and its connection helper left the bundled
            // stdlib for the TCP substrate (10 fns, 7 of them hooks, one
            // of them invoked unboundedly) and `std::io::tcp::stop`'s body
            // joined it; 430 since the TCP `Listener` followed them (its
            // three hooks, its default callback and the connection helper
            // that calls it, those two invoked unboundedly); 429 since the
            // JSON escaper appends runs (its two hex-string helpers went,
            // `__json_plain_byte` came); one fewer invoked unboundedly since
            // the HTTP client's `__http_retry_sentinel`, called in its retry
            // loop, went (`__http_retries_exhausted` came); 430 since
            // `std::process::close_stdin` came; 433 since `run_with`,
            // `spawn_with` and the deadline helper behind `wait` came; one
            // more fn and one more invoked unboundedly since the connect
            // deadlines (#1538); 435, and one more invoked unboundedly,
            // since the HTTP client writes its `Host` header through
            // `__http_host_text`; 436 since `std::hex` came (its three fns)
            // and the secret and JSON digit helpers it replaced went; 437
            // since `std::test::assert_near` came; 441 since `std::url`
            // came (its four fns; three more invoked unboundedly:
            // `query_param`'s pair loop decodes through them); 446 since
            // `std::rand::Rng` came (its birth, an entry point, and four
            // methods; `next` invoked unboundedly by `next_int`'s rejection
            // loop); 447 since `std::crypto::ed25519_keypair`'s body came;
            // 458, and one more entry point, since std::http's streamed body
            // came (`BodyWriter`'s birth, `chunk` and `end`, `SseWriter`'s
            // `event` and `end`, and six framing helpers); 471, and four
            // more invoked unboundedly, since the HTTP client's call
            // deadline came (eight helpers, five of them called in its read
            // and retry loops, and its sends no longer through
            // `__http_send_raw`); 476, and five more invoked
            // unboundedly, since `std::log`'s sinks write records (the
            // format, level, time, render and JSON-fields helpers, each
            // called from a sink's bus handler); 478, and two more, since
            // a record's values are escaped byte by byte (the plain-byte
            // and hex-digit helpers); 480 since `std::json::Value` came
            // (`parse` and `null`); 481, and one more entry point, since
            // `std::observe::Probes` came (its birth); 483, and one more
            // invoked unboundedly, since the stdlib's own terminal lines
            // are written by `log.hl` (`__std_log_diag`, `__std_log_out`);
            // 484, and one more invoked unboundedly, since
            // `Registry.render()` renders a series at a time
            // (`__metrics_render_series`); 486, and two more entry points,
            // since a `std::metrics::Registry` registers at birth and leaves
            // at dissolve (the observe backends themselves join only a
            // program that holds one)
            changed_lines(&old.render(), &now.render()),
            [
                "- # 486 fns, 32 entry points, 132 invoked-unboundedly",
                "+ # 486 fns, 32 entry points, 131 invoked-unboundedly",
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
        let rows_now = &whole_effects(snap);
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

/// `invoked` counts the copy's fns too: two more since F.40 E5, which
/// resolves the copy's indirect calls to the function values of their
/// type, so the router's fn-route dispatch reaches `__http_fn_unset` and
/// the listener's `on_conn` reaches `__default_on_connection`. One fewer
/// since the HTTP server left the copy for the TCP substrate, appended to
/// a program that names it: its accept loop's hand-off is no copy fn. Two
/// fewer since the TCP `Listener` followed it: `__handle_one_connection`
/// and the `__default_on_connection` its `on_conn` reaches are no copy
/// fns either. One fewer since the HTTP client's retry loop records its
/// failure in place: `__http_retry_sentinel`, called in that loop, went.
/// One more since the client writes its `Host` header through
/// `__http_host_text` (an IPv6 host is bracketed), called per request.
/// Four more since the client's call deadline came: its arming, reads,
/// writes and deadline tests run in its read and retry loops, and its
/// writes go to the deadline sends rather than `__http_send_raw` (and
/// oidc, which calls the one-shot fns by their public path, one more of
/// them beside the copy). Five more since `std::log`'s sinks write
/// records: the record helpers each sink's bus handler calls. Two more
/// since a record's values are escaped byte by byte.
/// And the copy's fns a program calls by their public path
/// (`std::…`) run where they are called, so a bus
/// handler's such calls invoke them unboundedly beside the copy: is_route
/// six of them, oidc ten more. None is the program's own.
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
        pinned_unreached(t, (133, 0), &["Gateway::handle", "RouterV1::route"], &[], &lines);
    }
}

/// 90-unowned-literal-positions declares its own `Server` with a
/// `handle`, and never starts the stdlib's.
#[test]
fn reached_unowned_literal_positions() {
    pinned_unreached(
        "crates/hale-codegen/tests/fixtures/examples/90-unowned-literal-positions",
        (133, 0),
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
        (155, 13),
        &[
            "b64",
            "decode",
            "form_field",
            "printable",
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
            "- fn printable   [invoked-unboundedly, scratch-local]",
            "+ fn printable   [scratch-local]",
            "- fn signed_id_token   [invoked-unboundedly]",
            "+ fn signed_id_token",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @5553..5602",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @5553..5602",
            "- fn StubIssuer::handle   [invoked-unboundedly]",
            "+ fn StubIssuer::handle",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @7970..8290",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @7970..8290",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @8093..8275",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @8093..8275",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @9000..9134",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @9000..9134",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @9392..9535",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @9392..9535",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @10051..10273",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @10051..10273",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @10171..10232",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve @10171..10232",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @11155..11341",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @11155..11341",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @11641..11844",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @11641..11844",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @13097..13266",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @13097..13266",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @13293..13347",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @13293..13347",
            "- fn StubIssuer::json   [invoked-unboundedly]",
            "+ fn StubIssuer::json",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @6842..6974",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @6842..6974",
            "- fn StubIssuer::service_token   [invoked-unboundedly]",
            "+ fn StubIssuer::service_token",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve @14189..14359",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve @14189..14359",
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
        (135, 6),
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
        // Isolate the run-to-exit correction from E3b on both sides.
        // E3b's corpus pins separately cover the new caller-arena sites.
        let now = &without_frames(now);
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

/// A run-to-exit program loses the copy's leak sites and its own. The
/// copy's used to be 21 in every one: each a value a copy fn builds in
/// its own loop and returns (`__md_to_html`, the YAML reader's
/// `list_item`, …), which GH #1490 (C2) judges once per call, so in a
/// program that reaches none of them from a long-lived frame they are no
/// leak sites at all, and the 84 programs that lost only those no longer
/// move. What is left is the own `main` in-loop sites (the two programs
/// that start `std::io::tcp::Listener` moved too while it was the copy's,
/// and are no run-to-exit programs since it is the TCP connection
/// substrate's); no other target moves.
#[test]
fn own_entries_run_to_exit_programs() {
    let mut moved = 0;
    let mut own = Vec::new();
    let mut more: Vec<(String, usize)> = Vec::new();
    for t in targets() {
        let Some((copy, mine)) = run_to_exit(&t) else { continue };
        moved += 1;
        if copy != 0 {
            more.push((t.clone(), copy));
        }
        own.extend(mine.into_iter().map(|s| format!("{t}: {s}")));
    }
    // 81: GH #1076's `tests/hale/unit_declarations_test.hl` is one more,
    // losing the copy's 21 like the rest; 82: U2's
    // `tests/hale/identities_and_ranges_test.hl` is one more again; 83:
    // U3's `tests/hale/unit_quantities_test.hl`; 88: U5's five acceptance
    // programs, `tests/hale/units_{identities,money,temperature,ticks,time}_test.hl`;
    // 89: `tests/hale/float_widening_surfaces_test.hl`; 90:
    // `tests/hale/time_return_test.hl`; 89: `dna/api`, whose `Head::run` serves a
    // surface and is no longer a run-to-exit program (R4 B); 5: GH #1490
    // (C2), the copy's 21 returned loop accumulators no longer leak; 3:
    // `docs-server` and `http-hello` start `std::io::tcp::Listener`, a
    // listener of the TCP connection substrate whose `run` is the
    // program's, as `std::http::Server`'s is: neither is run-to-exit; 4:
    // `tests/hale/json_escape_runs_test.hl`, the property test of the
    // run-appending JSON escaper; 5:
    // `tests/hale/enum_return_copy_loops_test.hl`, whose decoder builds a
    // record per element of a JSON array; 6:
    // `tests/hale/bounded_whole_assign_test.hl`, whose `fill` builds a
    // String per pushed element; 7: `tests/hale/api_codec_call_test.hl`,
    // whose codec, called by the program itself, decodes the same way; 8:
    // `tests/hale/api_codec_import_test.hl`, the same codec of an imported
    // type; 9: `tests/hale/api_value_map_test.hl`, whose map of values builds
    // a record per member it reads; 10: `tests/hale/api_patch_field_test.hl`,
    // whose codec of a presence triple decodes the same way; 11:
    // `tests/hale/api_json_value_test.hl`, whose codec decodes a JSON value's
    // record the same way.
    assert_eq!(moved, 11, "the run-to-exit programs among the targets that lose a site");
    assert_eq!(more, [], "the copy's leak sites");
    assert_eq!(
        own,
        [
            // `tests/hale/api_codec_call_test.hl`: the api codec the program calls
            // itself, its decoders' refusals and encoders' appends (a record's,
            // a wire map's keyed record's, and those of a record holding an
            // enum and an optional map)
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"JsonError\") @33693..33738 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"JsonError\") @33913..33958 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"JsonError\") @34100..34148 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"Check\") @34163..34196 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Desk StructLit(\"JsonError\") @23134..23181 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Desk StructLit(\"JsonError\") @24385..24431 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") @4718..4764 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") @5223..5269 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") @5354..5400 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") @5542..5591 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") @5621..5670 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"Line\") @5685..5720 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") @9526..9571 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") @10898..10944 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") @11444..11492 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") @12135..12180 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Check__by_name StringConcat @34296..34331 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_encode_Check__by_name StringConcat @34343..34352 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25368..25377 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25395..25485 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25395..25450 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25395..25442 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25395..25405 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25722..25731 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25749..25839 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25749..25804 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25749..25796 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat @25749..25759 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Line StringConcat @5809..5875 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_encode_Line StringConcat @5887..5925 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_encode_Line StringConcat @5937..5946 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_encode_Order StringConcat @13675..13684 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Order StringConcat @13702..13731 InUnboundedLoop",
            // `tests/hale/api_codec_import_test.hl`: the codec of an imported type
            "tests/hale/api_codec_import_test.hl: __api_decode___lib_tests__hale__api_codec_import_lib___main__Item StructLit(\"JsonError\") @3439..3484 InUnboundedLoop",
            "tests/hale/api_context_test.hl: main StringConcat @13548..13655 InUnboundedLoop",
            "tests/hale/api_context_test.hl: main StringConcat @13548..13631 InUnboundedLoop",
            // GH #1490 (step 3): `self.buf = self.buf + chunk`, an
            // append whose operands are typed by the field and the
            // `recv` result, not a literal.
            "tests/hale/api_context_test.hl: Api::ask StringConcat @6460..6476 InvokedUnboundedly",
            // `tests/hale/api_json_value_test.hl`: its codec of JSON values, the
            // decoders' refusals and a list's appends
            "tests/hale/api_json_value_test.hl: __api_decode_Change StructLit(\"JsonError\") @8152..8201 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Change StructLit(\"JsonError\") @8770..8818 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Log StructLit(\"JsonError\") @13702..13752 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Log StructLit(\"JsonError\") @13812..13860 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Seat StructLit(\"JsonError\") @4003..4052 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_encode_Log StringConcat @14372..14381 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_encode_Log StringConcat @14399..14414 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: main StringConcat @3064..3136 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: nested StringConcat @1054..1064 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: nested StringConcat @1082..1093 InUnboundedLoop",
            // `tests/hale/api_patch_field_test.hl`: its codec of presence triples,
            // a nested record's refusal
            "tests/hale/api_patch_field_test.hl: __api_decode_Inner StructLit(\"JsonError\") @4583..4627 InUnboundedLoop",
            "tests/hale/api_roles_test.hl: main StringConcat @10729..10754 InUnboundedLoop",
            "tests/hale/api_roles_test.hl: main StringConcat @10729..10747 InUnboundedLoop",
            "tests/hale/api_roles_test.hl: main StringConcat @10769..10823 InUnboundedLoop",
            // `tests/hale/api_value_map_test.hl`: its codec of a map of values,
            // a record built per member read and each value written
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Check\") @31577..31611 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @31621..31668 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @32985..33034 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Gauge\") @33120..33153 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @33163..33210 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @34527..34576 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Price\") @34670..34704 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @34714..34761 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Mark\") @36102..36134 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @36144..36190 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @37673..37722 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Stamp\") @37767..37797 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @37807..37854 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Tag\") @39174..39205 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") @39215..39260 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Level StructLit(\"JsonError\") @42816..42860 InvokedUnboundedly",
            "tests/hale/api_value_map_test.hl: __api_decode_Level StructLit(\"JsonError\") @43118..43162 InvokedUnboundedly",
            "tests/hale/api_value_map_test.hl: __api_decode_Level StructLit(\"JsonError\") @43179..43223 InvokedUnboundedly",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40603..40612 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40669..40771 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40669..40764 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40669..40731 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40669..40724 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40669..40716 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40669..40679 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @40965..40974 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41027..41100 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41027..41082 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41027..41074 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41027..41037 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41294..41303 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41359..41442 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41359..41414 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41359..41406 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41359..41369 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41632..41641 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41696..41778 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41696..41751 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41696..41743 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41696..41706 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @41970..41979 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42030..42126 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42030..42119 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42030..42092 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42030..42085 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42030..42077 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42030..42040 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42312..42321 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42377..42479 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42377..42472 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42377..42439 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42377..42432 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42377..42424 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat @42377..42387 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat @2167..2203 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat @2167..2197 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat @2167..2186 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat @2167..2180 InUnboundedLoop",
            "tests/hale/bounded_whole_assign_test.hl: fill StringConcat @1534..1552 InUnboundedLoop",
            "tests/hale/chains_tranche2_test.hl: main CollectionInsert(\"vec\") @1882..1910 InUnboundedLoop",
            "tests/hale/chains_tranche2_test.hl: main CollectionInsert(\"vec\") @2095..2120 InUnboundedLoop",
            // a record per element of the array the shelf's decoder reads
            "tests/hale/enum_return_copy_loops_test.hl: book StructLit(\"Book\") @972..1095 InvokedUnboundedly",
            // the property test's copy of the per-byte escaper as it was:
            // its `\u00XX` digits were a concatenation per control byte
            "tests/hale/json_escape_runs_test.hl: old_hex_byte StringConcat @7937..7972 InvokedUnboundedly",
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
    assert_eq!(sharing, 32, "the stdlib source files that share a name with the copy");
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
            // What the appended api runtime declares (`rpc_expand`: a file that
            // names it has it appended, as a program that serves does) is the
            // stdlib's, never the file's own: `is_runtime_pos` says which.
            let runtime: Vec<String> = bundle
                .programs
                .values()
                .flat_map(|p| p.items.iter())
                .filter_map(|i| match i {
                    hale_syntax::ast::TopDecl::Fn(f) => Some(&f.name),
                    hale_syntax::ast::TopDecl::Locus(l) => Some(&l.name),
                    hale_syntax::ast::TopDecl::Interface(n) => Some(&n.name),
                    _ => None,
                })
                .filter(|n| hale_types::rpc_expand::is_runtime_pos(n.span.start.0))
                .map(|n| n.name.clone())
                .collect();
            for (k, f) in &alone.fns {
                let shown = k.display();
                if runtime.iter().any(|n| shown == *n || shown.starts_with(&format!("{n}::"))) {
                    continue;
                }
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

/// The free fns and the loci the programs declare inside a
/// `module { … }`, at any depth.
fn module_nested<'a>(programs: impl IntoIterator<Item = &'a hale_syntax::ast::Program>) -> (BTreeSet<String>, BTreeSet<String>) {
    use hale_syntax::ast::{flat_decls, TopDecl};
    let (mut fns, mut loci) = (BTreeSet::new(), BTreeSet::new());
    for p in programs {
        for item in &p.items {
            let TopDecl::Module(m) = item else { continue };
            for d in flat_decls(&m.items) {
                match d {
                    TopDecl::Fn(f) => {
                        fns.insert(f.name.name.clone());
                    }
                    TopDecl::Locus(l) => {
                        loci.insert(l.name.name.clone());
                    }
                    _ => {}
                }
            }
        }
    }
    (fns, loci)
}

/// The summary before module-nested bodies were collected: `now`
/// without their rows, each call into one unresolved by its bare name.
/// Returns it, the rows it drops and the calls it unresolves, as
/// `caller -> callee`.
fn without_module_bodies(now: &AllocSummary, (fns, loci): &(BTreeSet<String>, BTreeSet<String>)) -> (AllocSummary, Vec<String>, Vec<String>) {
    let nested = |k: &FnKey| now.is_own(k) && k.locus.as_ref().map_or(fns.contains(&k.fn_name), |l| loci.contains(l));
    let rows = now.fns.keys().filter(|k| nested(k)).map(FnKey::display).collect();
    let mut old = now.clone();
    old.fns.retain(|k, _| !nested(k));
    let mut calls = Vec::new();
    for (key, f) in old.fns.iter_mut() {
        for c in &mut f.calls {
            let Callee::Resolved(k) = &c.callee else { continue };
            if nested(k) {
                calls.push(format!("{} -> {}", key.display(), k.display()));
                c.callee = Callee::Unresolved(k.fn_name.clone());
            }
        }
    }
    (old, rows, calls)
}

/// The dump's count line.
fn counts(summary: &AllocSummary) -> String {
    summary.render().lines().find(|l| l.contains(" fns, ")).unwrap_or_default().to_string()
}

/// Of the targets only 91-module-decls declares a body inside a module.
#[test]
fn module_bodies_only_one_target_has_them() {
    let mut with = Vec::new();
    for t in targets() {
        let nested = over_target(&t, |snap, _| {
            let (fns, loci) = module_nested(snap.bundle().programs.values().copied());
            !fns.is_empty() || !loci.is_empty()
        });
        if nested == Some(true) {
            with.push(t);
        }
    }
    assert_eq!(with, ["crates/hale-codegen/tests/fixtures/examples/91-module-decls"]);
}

/// 91-module-decls: seven rows, the seven calls into them resolve, the
/// effect rows gain the seven and `App::run`'s targets, no effect class
/// moves, and the model's holes close.
#[test]
fn module_bodies_91_module_decls() {
    over_target("crates/hale-codegen/tests/fixtures/examples/91-module-decls", |snap, now| {
        let bundle = snap.bundle();
        let (old, rows, calls) = without_module_bodies(now, &module_nested(bundle.programs.values().copied()));
        assert_eq!(
            rows,
            ["larger", "manhattan", "read_through", "twice", "Counter::bump", "Listener::on_ping", "Sensor::value"]
        );
        assert_eq!(
            calls,
            [
                "App::run -> manhattan",
                "App::run -> twice",
                "App::run -> larger",
                "App::run -> Sensor::value",
                "App::run -> read_through",
                "App::run -> Counter::bump",
                "App::run -> Counter::bump",
            ]
        );
        assert_eq!(
            (counts(&old), counts(now)),
            (
                "# 3 fns, 2 entry points, 0 invoked-unboundedly".to_string(),
                "# 10 fns, 3 entry points, 1 invoked-unboundedly".to_string()
            )
        );
        let rows_now = &whole_effects(snap);
        let rows_old = derive_effect_rows(&bundle, snap.demand_scope().expect("the scope"), Arc::new(old));
        let mut moved = Vec::new();
        for (k, r) in &rows_now.rows {
            let Some(o) = rows_old.rows.get(k) else { continue };
            assert_eq!((o.effects, o.unknown), (r.effects, r.unknown), "{}: an effect moved", k.display());
            if o.targets != r.targets {
                moved.push(k.display());
            }
        }
        assert_eq!(moved, ["App::run"]);
        assert_eq!(rows_now.rows.len(), rows_old.rows.len() + 7);
        assert!(snap.demand_effect_certificates().expect("the certificates").is_empty());
        let model = snap.demand_model().expect("the model");
        assert!(!model.holes.iter().any(|h| h.kind == hale_model::HoleKind::UnanalyzedBody));
        assert!(model.entities.loci.iter().all(|l| l.analyzable));
        let caps = &model.capabilities;
        assert!(caps.exact_calls && caps.exact_publishes && caps.exact_effects);
    })
    .expect("91-module-decls loads");
}

/// A module-nested fn's effects reach its caller's row: `App::run`
/// calls the module's `danger`, which runs a process. The old row was
/// `{publish, alloc}` and claimed to know everything.
#[test]
fn module_bodies_reach_the_effect_rows() {
    let src = r#"
module inner {
    fn danger() {
        let out = std::process::run("true") or raise;
        let c = out.code;
    }
}
type Tick { n: Int = 0; }
locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "m.t" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish "m.t" of type Tick; }
    run() {
        danger();
        "m.t" <- Tick { n: 1 };
    }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let now = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    let (old, rows, calls) = without_module_bodies(&now, &module_nested([&program]));
    assert_eq!((rows, calls), (vec!["danger".to_string()], vec!["App::run -> danger".to_string()]));
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let run = FnKey::method(None, "App", "run");
    let row = |s: AllocSummary| {
        let r = &derive_effect_rows(&bundle, &top, Arc::new(s)).rows[&run];
        (r.effects, r.unknown)
    };
    use hale_types::stdlib_surface::EffectSet;
    let both = EffectSet::PUBLISH.union(EffectSet::ALLOC);
    assert_eq!(row(old), (both, false), "the old row: no process, nothing unknown");
    assert_eq!(row(now), (both.union(EffectSet::SYSCALL).union(EffectSet::BLOCK), false));
}

/// A `causes:` law on a module-nested fn is certified: the walk sees
/// `poke`'s body. Before, the law "cannot be certified": `poke` was an
/// unanalyzed body, and the check said so.
#[test]
fn module_bodies_certify_a_causes_law() {
    let src = r#"
effect money;
module billing {
    @effects(causes: { money })
    fn poke(v: Int) -> Int { return v; }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let diags: Vec<String> = entries::check_program(&program).iter().map(|d| d.message.clone()).collect();
    assert_eq!(diags, Vec::<String>::new());
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let model = entries::derive_application_model(&bundle);
    assert!(!model.holes.iter().any(|h| h.kind == hale_model::HoleKind::UnanalyzedBody));
}

/// What the startable correction (GH #1490) removes in one target: its
/// leak sites gone, of the program's own fns, counted by the file the
/// owner's site is in. `None` when it moves nothing. Panics if it adds a
/// site, or moves one in a program with no `main` or whose own seed
/// declares a `run` hook or a bus handler on a locus it starts.
fn startable_moves(target: &str) -> Option<Vec<String>> {
    over_target_as_built(target, |_, now| {
        let now = &without_frames(now);
        let mut old = now.clone();
        old.startable = None;
        let site = |l: &LeakSite| {
            format!("{} {:?} @{}..{} {:?}", l.owner.display(), l.kind, l.span.start.0, l.span.end.0, l.reason)
        };
        let is: BTreeSet<String> = now.leak_sites().iter().map(site).collect();
        let was: BTreeSet<String> = old.leak_sites().iter().map(site).collect();
        assert!(is.is_subset(&was), "{target}: the correction added a leak site");
        if is == was {
            return None;
        }
        let started = now.startable.as_ref().expect("a site moved, so the set is known");
        let live = now.fns.values().any(|f| {
            now.is_own(&f.key)
                && matches!(f.entry, Some(EntryKind::Run) | Some(EntryKind::BusHandler))
                && f.key.locus.as_ref().is_none_or(|l| started.contains(l))
        });
        assert!(!live, "{target}: a site moved in a program that starts a long-lived locus");
        assert!(is.is_empty(), "{target}: the program runs to exit and keeps a leak site");
        let mut by_seed: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for l in old.leak_sites() {
            let owner = l.owner.display();
            let seed = match owner.strip_prefix("__lib_").and_then(|r| r.split("___").next()) {
                Some(seed) => seed.to_string(),
                None if now.is_own(&l.owner) => "the program's own".to_string(),
                None => "the stdlib's copy".to_string(),
            };
            *by_seed.entry(seed).or_default() += 1;
        }
        Some(by_seed.into_iter().map(|(s, n)| format!("{s}: {n}")).collect())
    })
    .flatten()
}

/// **An imported seed's loci run where they are named (GH #1490).** The
/// run-to-exit rule counted every `run` hook and bus handler of the
/// program's own fns, an imported seed's included, whether or not the
/// program ever started the locus that declares it: a program over
/// `dna/core` (69 bus handlers, seven `run` hooks) was long-lived however
/// it used the core, and the host's one-shot verbs, which start none of
/// them, were judged as a daemon. Now a hook or a handler of an imported
/// locus counts only when the program can start the locus
/// (`AllocSummary::startable`: its own seed's loci, and every imported
/// one a fn its `main` reaches, or a started locus, names; the stdlib's
/// loci it names count too, so `dna/ui`, which starts
/// `std::http::Server`, stays long-lived). Among the targets only the
/// host's verb program moves: it starts no locus with a hook or a
/// handler, so it runs to exit, and every leak site of its fns goes —
/// counted by the seed the fn is declared in (the host's 652, the core's
/// 191, …; GH #1490's step 3 typed more concatenations, 565 and 145
/// before it and before init --with's planner, and step 4 made a String field replaced whole with a value
/// proven independent of it store-latest). The correction only narrows: no target gains a site. The old
/// answer is the corrected summary with `startable` cleared.
#[test]
fn startable_only_the_verb_program_runs_to_exit() {
    let mut moved = Vec::new();
    for t in targets() {
        if let Some(m) = startable_moves(&t) {
            moved.push((t, m));
        }
    }
    let strs = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
    assert_eq!(
        moved,
        vec![(
            "dna/host/verb".to_string(),
            strs(&[
                "dna__core: 222",
                "dna__core__pond__pq: 16",
                "dna__core__pond__realtime__nats: 2",
                "dna__host: 617",
                "dna__operations: 135",
                "the stdlib's copy: 1",
            ])
        )]
    );
}
