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

#[path = "support/site_keys.rs"]
mod site_keys;
use site_keys::{leak_key, stable_dump};
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
            // program that holds one); 491 since std::crypto's secp256k1
            // bodies came (`secp256k1_public_key`, `secp256k1_sign`,
            // `secp256k1_recover` and their failure-code helper, GH #1929);
            // 511 since `wide.hl` came (GH #1934: `std::u256` /
            // `std::i256`'s fallible calls and the JSON codecs' hex
            // helpers, 20 fns); 513 since `std::json::Builder` writes
            // JSON as given in `raw_field` and `raw_value` (GH #1943);
            // 515 since `time_rfc3339.hl` came (GH #1948:
            // `std::time::parse_rfc3339` and its digit helper); 525 since
            // `str_helpers.hl` came (GH #1995: std::str's nine splits and
            // trims and their whitespace helper); 533 since the text `for`
            // sources' eight cursor hooks came (GH #1995 S2); 534 since
            // `net.hl` came (GH #1908: `std::net::resolve`); 535 since a
            // chain over a text source compares a part with the best one in
            // place (`__str_view_cmp`, GH #1995 S2, chain sources)
            changed_lines(&stable_dump(&old.render()), &stable_dump(&now.render())),
            [
                "- # 535 fns, 32 entry points, 132 invoked-unboundedly",
                "+ # 535 fns, 32 entry points, 131 invoked-unboundedly",
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
                .map(|l| leak_key(s, l))
                .collect()
        };
        let (lw, li) = (leaks(&old), leaks(now));
        assert!(li.is_subset(&lw), "{target}: the correction added a leak site");
        Unreached {
            own: was.difference(&is).filter(|k| !stdlib.contains(k)).map(FnKey::display).collect(),
            leaks: lw.difference(&li).cloned().collect(),
            invoked: (was.len(), is.len()),
            lines: own_changed_lines(&stable_dump(&old.render()), &stable_dump(&now.render()), &names),
        }
    })
}

/// The five targets pinned below are the only ones whose own fns or leak
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
            "crates/hale-codegen/tests/fixtures/examples/90-unowned-literal-positions",
            "dna/oidc",
            "tests/hale/codegen_gaps_test.hl",
            "tests/hale/dot_variant_patterns_test.hl",
            "tests/hale/is_route_test.hl",
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

/// 90-unowned-literal-positions declares its own `Server` with a
/// `handle`, and never starts the stdlib's.
#[test]
fn reached_unowned_literal_positions() {
    pinned_unreached(
        "crates/hale-codegen/tests/fixtures/examples/90-unowned-literal-positions",
        (133, 0),
        &["Provider::submit", "Server::handle"],
        &["Provider::submit CollectionInsert(\"vec\") #1 InvokedUnboundedly"],
        &[
            "- fn Provider::submit   [invoked-unboundedly]",
            "+ fn Provider::submit",
            "- alloc vec-insert       escaping=self-store  ACCUMULATES-UNBOUNDED  reclaim@locus-dissolve #1  <-- LEAK",
            "+ alloc vec-insert       escaping=self-store  once-per-invocation    reclaim@locus-dissolve #1",
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
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #5",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve #5",
            "- fn StubIssuer::handle   [invoked-unboundedly]",
            "+ fn StubIssuer::handle",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #2",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #2",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #4",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve #4",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #3",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #3",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #4",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #4",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #6",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #6",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #13",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve #13",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #8",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #8",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #9",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #9",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #12",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #12",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #13",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #13",
            "- fn StubIssuer::json   [invoked-unboundedly]",
            "+ fn StubIssuer::json",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- fn StubIssuer::service_token   [invoked-unboundedly]",
            "+ fn StubIssuer::service_token",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #2",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #2",
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
        &["Api::handle StructLit(\"std::http::Response\") #1 InvokedUnboundedly"],
        &[
            "- fn Api::handle   [invoked-unboundedly]",
            "+ fn Api::handle",
            "- alloc struct std::http::Response escaping=return      ACCUMULATES-UNBOUNDED  reclaim@locus-dissolve #1  <-- LEAK",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- fn Api::list   [invoked-unboundedly]",
            "+ fn Api::list",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- fn Api::rename   [invoked-unboundedly]",
            "+ fn Api::rename",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- fn Api::show   [invoked-unboundedly]",
            "+ fn Api::show",
            "- alloc struct std::http::Response escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc struct std::http::Response escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
            "- alloc string-concat    escaping=return      per-iteration-reclaim  reclaim@locus-dissolve #1",
            "+ alloc string-concat    escaping=return      once-per-invocation    reclaim@locus-dissolve #1",
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
        let site = |s: &AllocSummary, l: &LeakSite| leak_key(s, l);
        let is: BTreeSet<String> = now.leak_sites().iter().map(|l| site(now, l)).collect();
        let gone: Vec<LeakSite> = old.leak_sites().into_iter().filter(|l| !is.contains(&site(&old, l))).collect();
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
        let own = gone.iter().filter(|l| now.is_own(&l.owner)).map(|l| site(&old, l)).collect();
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
    // record the same way; 12: `tests/hale/api_encode_linear_test.hl`, whose
    // codec decodes its page of records the same way; 13:
    // `tests/hale/crypto_sha256_stream_test.hl`, whose `main` builds its
    // assertion messages and failure fallbacks in its loops; 14:
    // `tests/hale/api_decimal_field_test.hl`, whose codec of a Decimal row
    // decodes the same way; 15: `tests/hale/json_defaults_test.hl` and 16:
    // `tests/hale/json_enum_field_test.hl`, whose JSON codecs decode (and
    // the enum's writes) the same way; 17: `tests/hale/api_decimal_string_test.hl`
    // and 18: `tests/hale/api_positional_test.hl`, whose codecs (a Decimal
    // read from a string, a positional record's rows) decode the same way;
    // 19: `tests/hale/u256_arith_test.hl`, whose loop over refused texts
    // builds its messages, and 20: `tests/hale/u256_codec_test.hl`, whose
    // codecs of 256-bit integers decode the same way (GH #1934); 21:
    // `tests/hale/api_map_wrong_kind_test.hl`, whose wire maps decode the
    // same way (GH #1944); 22: `tests/hale/bounded_values_test.hl`, whose
    // fns build a returned bounded list's elements in their loops (GH #1951);
    // 23: `tests/hale/bounded_payload_enum_test.hl`, whose `fill` builds a
    // payload String per pushed element, as `bounded_whole_assign_test.hl`'s
    // does (GH #1951 item 4);
    // 24: `tests/hale/optional_field_test.hl`, whose codec of optional fields
    // decodes the same way (GH #1951 item 3); 25:
    // `tests/hale/string_append_test.hl`, whose `main` appends to Strings in
    // its loops (GH #1947); 26: `tests/hale/str_split_sources_test.hl`,
    // whose loops push the parts they visit into a vec (GH #1995 S2); 27:
    // `tests/hale/units_form_iteration_test.hl`, whose loops over a form's
    // cells build their assertion messages (GH #2026); 28:
    // `tests/hale/ring_buffer_push_flat_test.hl`, whose `main` builds the
    // records it pushes in its loops (GH #2024); 29:
    // `tests/hale/str_chain_sources_test.hl`, whose chains over text
    // sources push the parts into a vec the same way (GH #1995 S2, chain
    // sources); 30: `tests/hale/bounded_part_result_push_test.hl`, whose
    // `summary_of` builds the record it returns and whose `summaries`
    // stores each summary as it is (GH #2067; until GH #2099 the whole
    // assignment kept the row's elements and the push copied the summary); 31:
    // `tests/hale/bounded_inline_enum_result_push_test.hl`, whose `s_of`
    // builds the record `build` pushes as it is (GH #2088); 32:
    // `tests/hale/dot_variant_patterns_test.hl`, whose `total` passes a
    // String bound in each element's match to `len` (GH #2069).
    assert_eq!(moved, 32, "the run-to-exit programs among the targets that lose a site");
    assert_eq!(more, [], "the copy's leak sites");
    // A codec's sites lie in the JSON codecs' parse space, from
    // `JSON_SYNTH_BASE` (0x6000_0000 = 1610612736), past every file.
    assert_eq!(
        own,
        [
            // `tests/hale/api_codec_call_test.hl`: the api codec the program calls
            // itself, its decoders' refusals and encoders' per-element pieces (a record's,
            // a wire map's keyed record's, and those of a record holding an
            // enum and an optional map)
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Check__by_name StructLit(\"Check\") #1 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Desk StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Desk StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") #4 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"JsonError\") #5 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Line StructLit(\"Line\") #1 InvokedUnboundedly",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") #8 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_decode_Order StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat #4 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat #5 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat #8 InUnboundedLoop",
            "tests/hale/api_codec_call_test.hl: __api_encode_Desk StringConcat #9 InUnboundedLoop",
            // `tests/hale/api_codec_import_test.hl`: the codec of an imported type
            "tests/hale/api_codec_import_test.hl: __api_decode___lib_tests__hale__api_codec_import_lib___main__Item StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/api_context_test.hl: main StringConcat #41 InUnboundedLoop",
            "tests/hale/api_context_test.hl: main StringConcat #42 InUnboundedLoop",
            "tests/hale/api_context_test.hl: Api::ask StringConcat #2 InvokedUnboundedly",
            // `tests/hale/api_decimal_field_test.hl`: its codec of a Decimal row,
            // decoding the same way (the same sites; they moved when the
            // Decimal reader's text grew to read a quoted number, 2026-11-14)
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"JsonError\") #5 InvokedUnboundedly",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"JsonError\") #6 InvokedUnboundedly",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Line StructLit(\"Line\") #1 InvokedUnboundedly",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Order StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Order StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_decimal_field_test.hl: __api_decode_Order StructLit(\"JsonError\") #10 InUnboundedLoop",
            // `tests/hale/api_decimal_string_test.hl`: its codecs of Decimals read
            // from strings (and written as strings where tagged), the
            // decoders' refusals and the encoders' per-element pieces
            "tests/hale/api_decimal_string_test.hl: __api_decode_Order StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Order StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Order StructLit(\"JsonError\") #12 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Order StructLit(\"JsonError\") #13 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Order StructLit(\"JsonError\") #16 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"JsonError\") #12 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"JsonError\") #13 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"JsonError\") #20 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"Mark\") #1 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_decode_Quote StructLit(\"JsonError\") #21 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_encode_Order StringConcat #8 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_encode_Order StringConcat #9 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_encode_Quote StringConcat #5 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __api_encode_Quote StringConcat #6 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_parse_Order StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_parse_Order StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_parse_Order StructLit(\"JsonError\") #8 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_parse_Order StructLit(\"JsonError\") #9 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_parse_Order StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_to_json_Order StringConcat #8 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: __json_to_json_Order StringConcat #9 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: main StringConcat #6 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: main StringConcat #7 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: main StringConcat #8 InUnboundedLoop",
            "tests/hale/api_decimal_string_test.hl: main StringConcat #9 InUnboundedLoop",
            // `tests/hale/api_encode_linear_test.hl`: its page's decoder, the
            // refusals and the record built per element read
            "tests/hale/api_encode_linear_test.hl: __api_decode_Item StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Item StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Item StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Item StructLit(\"JsonError\") #4 InvokedUnboundedly",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Item StructLit(\"JsonError\") #5 InvokedUnboundedly",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Item StructLit(\"Item\") #1 InvokedUnboundedly",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Page StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_encode_linear_test.hl: __api_decode_Page StructLit(\"JsonError\") #6 InUnboundedLoop",
            // `tests/hale/api_json_value_test.hl`: its codec of JSON values, the
            // decoders' refusals and a list's appends
            "tests/hale/api_json_value_test.hl: __api_decode_Change StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Change StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Log StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Log StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: __api_decode_Seat StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: main StringConcat #17 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: nested StringConcat #1 InUnboundedLoop",
            "tests/hale/api_json_value_test.hl: nested StringConcat #2 InUnboundedLoop",
            // `tests/hale/api_map_wrong_kind_test.hl`: its wire maps' codecs (GH
            // #1944), a map of records, of positional records, and a positional
            // record holding one, decoding the same way
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Bar StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Bar StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Bar StructLit(\"Bar\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Board StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Cell__by_at StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Cell__by_at StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Cell__by_at StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Cell__by_at StructLit(\"JsonError\") #4 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Cell__by_at StructLit(\"Cell\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Doc StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Sym__by_symbol StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Sym__by_symbol StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Sym__by_symbol StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Sym__by_symbol StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __api_decode_Sym__by_symbol StructLit(\"Sym\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Cell__by_at StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Cell__by_at StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Cell__by_at StructLit(\"Cell\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Pair__by_k StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Pair__by_k StructLit(\"Pair\") #1 InvokedUnboundedly",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Row StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __json_parse_Tagged StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __json_to_json_Tagged StringConcat #2 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: __json_to_json_Tagged StringConcat #3 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: api StringConcat #3 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: api StringConcat #4 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: api StringConcat #5 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: api StringConcat #6 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: tagged StringConcat #3 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: tagged StringConcat #4 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: tagged StringConcat #5 InUnboundedLoop",
            "tests/hale/api_map_wrong_kind_test.hl: tagged StringConcat #6 InUnboundedLoop",
            // `tests/hale/api_patch_field_test.hl`: its codec of presence triples,
            // a nested record's refusal
            "tests/hale/api_patch_field_test.hl: __api_decode_Inner StructLit(\"JsonError\") #2 InUnboundedLoop",
            // `tests/hale/api_positional_test.hl`: its positional records' codecs,
            // each row's refusals and the record built per row read, and
            // the records holding lists and maps of rows
            "tests/hale/api_positional_test.hl: __api_decode_Board StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #4 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #5 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #7 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #8 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #9 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #10 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #11 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #12 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #13 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #14 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #15 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #16 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #17 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #18 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #19 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #20 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #21 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #22 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #23 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"JsonError\") #24 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Candle StructLit(\"Candle\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #5 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #6 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #8 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #9 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #10 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"JsonError\") #11 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Level StructLit(\"Level\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Page StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Page StructLit(\"JsonError\") #9 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Page StructLit(\"JsonError\") #13 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Tick__by_sym StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Tick__by_sym StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Tick__by_sym StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Tick__by_sym StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Tick__by_sym StructLit(\"JsonError\") #5 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Tick__by_sym StructLit(\"Tick\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __api_decode_Trade StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_decode_Trade StructLit(\"JsonError\") #13 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_encode_Board StringConcat #2 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_encode_Board StringConcat #3 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_encode_Trade StringConcat #6 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __api_encode_Trade StringConcat #7 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #4 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #6 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #8 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #9 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #10 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #12 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #13 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #14 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #15 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #16 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"JsonError\") #17 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Candle StructLit(\"Candle\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #4 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #6 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"JsonError\") #7 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Level StructLit(\"Level\") #1 InvokedUnboundedly",
            "tests/hale/api_positional_test.hl: __json_parse_Page StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Page StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: __json_parse_Page StructLit(\"JsonError\") #9 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: main StringConcat #7 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: main StringConcat #8 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: main StringConcat #9 InUnboundedLoop",
            "tests/hale/api_positional_test.hl: main StringConcat #10 InUnboundedLoop",
            "tests/hale/api_roles_test.hl: main StringConcat #28 InUnboundedLoop",
            "tests/hale/api_roles_test.hl: main StringConcat #29 InUnboundedLoop",
            "tests/hale/api_roles_test.hl: main StringConcat #30 InUnboundedLoop",
            // `tests/hale/api_value_map_test.hl`: its codec of a map of values,
            // a record built per member read and each value written
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Check\") #1 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #10 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Gauge\") #1 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #16 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Price\") #1 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #17 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Mark\") #1 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #22 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #28 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Stamp\") #1 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #29 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"Tag\") #1 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Health StructLit(\"JsonError\") #34 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_decode_Level StructLit(\"JsonError\") #1 InvokedUnboundedly",
            "tests/hale/api_value_map_test.hl: __api_decode_Level StructLit(\"JsonError\") #2 InvokedUnboundedly",
            "tests/hale/api_value_map_test.hl: __api_decode_Level StructLit(\"JsonError\") #3 InvokedUnboundedly",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #2 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #3 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #4 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #5 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #7 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #8 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #10 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #11 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #13 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #14 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #16 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #17 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #18 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #19 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #21 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #22 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #23 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: __api_encode_Health StringConcat #24 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat #5 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat #6 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat #7 InUnboundedLoop",
            "tests/hale/api_value_map_test.hl: main StringConcat #8 InUnboundedLoop",
            // `tests/hale/bounded_inline_enum_result_push_test.hl`: the record
            // `s_of` builds and `build` pushes as it is, no `CellCopy` beside
            // it, since its payload-free enum field is an inline tag (GH #2088)
            "tests/hale/bounded_inline_enum_result_push_test.hl: s_of StructLit(\"S\") #1 InvokedUnboundedly",
            "tests/hale/bounded_part_result_push_test.hl: summary_of StructLit(\"Sum\") #1 InvokedUnboundedly",
            // `tests/hale/bounded_payload_enum_test.hl`: the text its `show` and
            // `tags` build for every element they are handed (GH #1951)
            "tests/hale/bounded_payload_enum_test.hl: show StringConcat #1 InvokedUnboundedly",
            "tests/hale/bounded_payload_enum_test.hl: show StringConcat #2 InvokedUnboundedly",
            "tests/hale/bounded_payload_enum_test.hl: show StringConcat #5 InvokedUnboundedly",
            "tests/hale/bounded_payload_enum_test.hl: show StringConcat #6 InvokedUnboundedly",
            "tests/hale/bounded_payload_enum_test.hl: show StringConcat #9 InvokedUnboundedly",
            "tests/hale/bounded_payload_enum_test.hl: tags StringConcat #1 InUnboundedLoop",
            // `tests/hale/bounded_values_test.hl`: fns that build a returned
            // bounded list's record and String elements per pass (GH #1951)
            "tests/hale/bounded_values_test.hl: items_of StructLit(\"Item\") #1 InUnboundedLoop",
            "tests/hale/bounded_values_test.hl: items_of StringConcat #1 InUnboundedLoop",
            "tests/hale/bounded_values_test.hl: tags_of StringConcat #1 InUnboundedLoop",
            "tests/hale/bounded_whole_assign_test.hl: fill StringConcat #1 InUnboundedLoop",
            "tests/hale/chains_tranche2_test.hl: main CollectionInsert(\"vec\") #5 InUnboundedLoop",
            "tests/hale/chains_tranche2_test.hl: main CollectionInsert(\"vec\") #6 InUnboundedLoop",
            // `tests/hale/crypto_sha256_stream_test.hl`: the failure fallbacks and
            // assertion messages of the incremental SHA-256's test, and the
            // hasher its loop over hashlib's pinned digests builds per size
            "tests/hale/crypto_sha256_stream_test.hl: data BytesLit #1 InvokedUnboundedly",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #1 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #2 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #3 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StructLit(\"std::crypto::Sha256\") #3 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #4 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #5 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #6 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main BytesLit #5 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #7 InUnboundedLoop",
            "tests/hale/crypto_sha256_stream_test.hl: main StringConcat #8 InUnboundedLoop",
            // `tests/hale/dot_variant_patterns_test.hl`: `total` passes the String
            // a `.Refused(why)` arm binds from each element to `len` in its loop
            "tests/hale/dot_variant_patterns_test.hl: total CellCopy(\"copy: a local bound from a place is passed to a call that may write through its parameter\") #1 InUnboundedLoop",
            // a record per element of the array the shelf's decoder reads
            "tests/hale/enum_return_copy_loops_test.hl: book StructLit(\"Book\") #1 InvokedUnboundedly",
            // `tests/hale/json_defaults_test.hl`: its JSON codec of a record with
            // defaults, decoding the same way, and since 2026-11-07 its api codec
            // too (null is absence through both); its Decimal field's reader grew
            // to read a quoted number (2026-11-14), moving the sites after it
            "tests/hale/json_defaults_test.hl: __api_decode_Instrument StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __api_decode_Instrument StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __api_decode_Order StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __api_decode_Order StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __api_decode_Order StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __api_decode_Order StructLit(\"JsonError\") #17 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __json_parse_Order StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/json_defaults_test.hl: __json_parse_Order StructLit(\"JsonError\") #5 InUnboundedLoop",
            // `tests/hale/json_enum_field_test.hl`: its JSON codec of a record with
            // an enum field, decoding and writing the same way
            "tests/hale/json_enum_field_test.hl: __json_parse_Ord StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/json_enum_field_test.hl: __json_to_json_Ord StringConcat #2 InUnboundedLoop",
            "tests/hale/json_enum_field_test.hl: __json_to_json_Ord StringConcat #3 InUnboundedLoop",
            // the property test's copy of the per-byte escaper as it was:
            // its `\u00XX` digits were a concatenation per control byte
            "tests/hale/json_escape_runs_test.hl: old_hex_byte StringConcat #1 InvokedUnboundedly",
            // `tests/hale/optional_field_test.hl`: its codec of optional fields
            // (GH #1951 item 3), decoding and writing the same way as a
            // hand-written presence pair's
            "tests/hale/optional_field_test.hl: __api_decode_Inner StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_decode_Ticker StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_decode_Ticker StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_decode_Ticker StructLit(\"JsonError\") #10 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_decode_Ticker StructLit(\"JsonError\") #13 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_decode_Ticker StructLit(\"JsonError\") #18 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_encode_Ticker StringConcat #20 InUnboundedLoop",
            "tests/hale/optional_field_test.hl: __api_encode_Ticker StringConcat #21 InUnboundedLoop",
            // `tests/hale/ring_buffer_push_flat_test.hl`: the record `take`
            // builds to push (GH #2024); the ones `main`'s loop builds are
            // reclaimed per iteration since GH #2094
            "tests/hale/ring_buffer_push_flat_test.hl: Names::take StructLit(\"Named\") #1 InvokedUnboundedly",
            // `tests/hale/str_chain_sources_test.hl` (GH #1995 S2, chain sources): the
            // pushes of its `into` / `sort_into` / `reverse_into` chains and its
            // control loop, and the `map` stage's concatenation, per part
            "tests/hale/str_chain_sources_test.hl: main CollectionInsert(\"vec\") #1 InUnboundedLoop",
            "tests/hale/str_chain_sources_test.hl: main CollectionInsert(\"vec\") #2 InUnboundedLoop",
            "tests/hale/str_chain_sources_test.hl: main StringConcat #1 InUnboundedLoop",
            "tests/hale/str_chain_sources_test.hl: main CollectionInsert(\"vec\") #3 InUnboundedLoop",
            "tests/hale/str_chain_sources_test.hl: main CollectionInsert(\"vec\") #4 InUnboundedLoop",
            "tests/hale/str_chain_sources_test.hl: main CollectionInsert(\"vec\") #5 InUnboundedLoop",
            "tests/hale/str_chain_sources_test.hl: words_by_loop CollectionInsert(\"vec\") #1 InUnboundedLoop",
            // `tests/hale/str_split_sources_test.hl`: the vec pushes that keep
            // each part a text source's loop visits (GH #1995 S2)
            "tests/hale/str_split_sources_test.hl: lines_of CollectionInsert(\"vec\") #1 InUnboundedLoop",
            "tests/hale/str_split_sources_test.hl: main CollectionInsert(\"vec\") #1 InUnboundedLoop",
            "tests/hale/str_split_sources_test.hl: main CollectionInsert(\"vec\") #2 InUnboundedLoop",
            "tests/hale/str_split_sources_test.hl: main CollectionInsert(\"vec\") #3 InUnboundedLoop",
            "tests/hale/str_split_sources_test.hl: main CollectionInsert(\"vec\") #4 InUnboundedLoop",
            "tests/hale/str_split_sources_test.hl: parts CollectionInsert(\"vec\") #1 InUnboundedLoop",
            "tests/hale/str_split_sources_test.hl: words_of CollectionInsert(\"vec\") #1 InUnboundedLoop",
            // `tests/hale/string_append_test.hl`: the appending assignments
            // its loops make, to a local and to a field (GH #1947)
            "tests/hale/string_append_test.hl: build StringConcat #1 InUnboundedLoop",
            "tests/hale/string_append_test.hl: build StringConcat #2 InUnboundedLoop",
            "tests/hale/string_append_test.hl: build StringConcat #3 InUnboundedLoop",
            "tests/hale/string_append_test.hl: build_reading StringConcat #1 InUnboundedLoop",
            "tests/hale/string_append_test.hl: build_reading StringConcat #2 InUnboundedLoop",
            "tests/hale/string_append_test.hl: build_reading StringConcat #3 InUnboundedLoop",
            "tests/hale/string_append_test.hl: Doc::build StringConcat #1 InUnboundedLoop",
            // `tests/hale/u256_arith_test.hl`: the parse refusals it loops over
            // build their assertion messages in the loop (GH #1934)
            "tests/hale/u256_arith_test.hl: conversions StringConcat #4 InUnboundedLoop",
            "tests/hale/u256_arith_test.hl: conversions StringConcat #5 InUnboundedLoop",
            "tests/hale/u256_arith_test.hl: conversions StringConcat #6 InUnboundedLoop",
            "tests/hale/u256_arith_test.hl: conversions StringConcat #7 InUnboundedLoop",
            "tests/hale/u256_arith_test.hl: conversions StringConcat #8 InUnboundedLoop",
            "tests/hale/u256_arith_test.hl: conversions StringConcat #9 InUnboundedLoop",
            "tests/hale/u256_arith_test.hl: conversions StringConcat #10 InUnboundedLoop",
            // `tests/hale/u256_codec_test.hl`: its codecs of 256-bit integers and
            // `json:",hex"` Ints, the decoders' refusals and the encoders'
            // per-element pieces (GH #1934)
            "tests/hale/u256_codec_test.hl: __api_decode_Account StructLit(\"JsonError\") #16 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Account StructLit(\"Holding\") #1 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Account StructLit(\"JsonError\") #23 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #4 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #7 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #8 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #13 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #14 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #15 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Receipt StructLit(\"JsonError\") #17 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_decode_Tick StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_encode_Account StringConcat #8 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_encode_Account StringConcat #9 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_encode_Account StringConcat #11 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_encode_Account StringConcat #12 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_encode_Account StringConcat #13 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __api_encode_Account StringConcat #14 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Account StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Account StructLit(\"Holding\") #1 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Account StructLit(\"JsonError\") #17 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #2 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #3 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #5 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #6 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #10 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #11 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_parse_Receipt StructLit(\"JsonError\") #12 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_to_json_Account StringConcat #8 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_to_json_Account StringConcat #9 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_to_json_Account StringConcat #11 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_to_json_Account StringConcat #12 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_to_json_Account StringConcat #13 InUnboundedLoop",
            "tests/hale/u256_codec_test.hl: __json_to_json_Account StringConcat #14 InUnboundedLoop",
            // `tests/hale/units_form_iteration_test.hl`: the f-strings its
            // loops over a form's cells build (GH #2026)
            "tests/hale/units_form_iteration_test.hl: main StringConcat #1 InUnboundedLoop",
            "tests/hale/units_form_iteration_test.hl: main StringConcat #2 InUnboundedLoop",
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
    // 34 since `time_rfc3339.hl` came (GH #1948: `std::time::parse_rfc3339`);
    // 35 since `str_helpers.hl` came (GH #1995: std::str's splits and trims);
    // 36 since `net.hl` came (GH #1908: std::net::resolve).
    assert_eq!(sharing, 36, "the stdlib source files that share a name with the copy");
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
        let site = |s: &AllocSummary, l: &LeakSite| leak_key(s, l);
        let is: BTreeSet<String> = now.leak_sites().iter().map(|l| site(now, l)).collect();
        let was: BTreeSet<String> = old.leak_sites().iter().map(|l| site(&old, l)).collect();
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
                // GH #1758: 9 more, the named habitats' resolver and
                // `developer_habitat_projects`' loop over the habitat
                // directories under `$HOME`
                "dna__core: 520",
                "dna__core__pond__pq: 16",
                "dna__core__pond__realtime__nats: 2",
                // GH #2094: 6 fewer in the host, the sites its free fns'
                // loops make and keep within an iteration
                // (`forgejo_healthy`'s poll, `append_lines`' row)
                "dna__host: 818",
                "dna__operations: 262",
                "the stdlib's copy: 1",
            ])
        )]
    );
}

/// GH #2069's test calls a `fn(Int) -> Int` payload a `.Use(f)` arm binds,
/// and starts no loop: as in GH #1942's below, the appended HTTP
/// dispatch's fallback site, one of that type's fn values, is no longer
/// counted as invoked without bound.
#[test]
fn reached_dot_variant_patterns_test() {
    pinned_unreached(
        "tests/hale/dot_variant_patterns_test.hl",
        (131, 0),
        &[],
        &["__http_fn_unset StructLit(\"std::http::Response\") #1 InvokedUnboundedly"],
        &[],
    );
}

/// GH #1942's test of the shapes now compiled starts no loop, so, as in
/// the perspective fixtures, the appended HTTP dispatch's fallback site is
/// no longer counted as invoked without bound.
#[test]
fn reached_codegen_gaps_test() {
    pinned_unreached(
        "tests/hale/codegen_gaps_test.hl",
        (131, 0),
        &[],
        &["__http_fn_unset StructLit(\"std::http::Response\") #1 InvokedUnboundedly"],
        &[],
    );
}
