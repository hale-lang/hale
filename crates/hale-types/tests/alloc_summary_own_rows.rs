//! The user-program readers read the one summary's own rows (F.40
//! phase 3, E3a 7 of 9), pinned per target.
//!
//! The model, the `@budget` engines and the artifact's rows used to
//! build a summary of their own: the checked programs alone, cross-seed
//! calls resolved through the import renames. They read the snapshot's
//! summary now (`Snapshot::demand_alloc_summary`), which holds the
//! stdlib's analysis copy beside the programs, through
//! `AllocSummary::own_rows`: the program's own fns and loci, and a call
//! into the copy as the unresolved call it is when the program is
//! summarized alone. The model is a user-program model and its
//! `shape_hash` is the build and replay identity, so the copy must not
//! move it: a handle call that now resolves into a copy body would
//! otherwise spend a dispatch-site ordinal and then be dropped, a gap
//! in `calls.dispatch_site` and a lost row (`dna/api`,
//! `api_binding_run_test`). What the model could gain from the copy's
//! rows is a separate, future correction.
//!
//! Over the 170 targets `hale check` is run over here (the corpus
//! fixtures, the `tests/hale` programs and the DNA seeds), the own rows
//! equal the program-alone summary in every field of every fn, on the
//! 169 that have a summary, across the program's edges that resolve
//! into the copy or dispatch through its interfaces (14,014 when this
//! landed): nothing the readers answer moves. So do they over every
//! corpus program checked alone (2,257 when this landed, the stdlib's
//! own source files among them).
//!
//! The frontier's `causes:` engine and the resource budget (E3a 8 of 9)
//! read the same own rows in place of the plain summary they built (no
//! stdlib, no renames). Over every corpus program checked alone the own
//! rows are that plain summary; over the 170 targets, renames included,
//! `--dump-resource-budget` and `--warn-resource-leak` are identical. The
//! own rows are what keeps the copy out: read whole, the copy's bodies
//! add 8 fd-acquiring sites to every program's budget, and a program
//! that starts the TCP `Listener` (`http-hello`) reaches its hooks.

use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::alloc_summary::{summarize_identified, AllocSummary, Callee};

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

/// Every field a reader can read, rendered: the program-alone summary
/// and the own rows must agree on each.
fn fields(s: &AllocSummary) -> Vec<(String, String)> {
    let mut v = vec![
        ("eager_only_loci".to_string(), format!("{:?}", s.eager_only_loci)),
        ("bounded_loci".to_string(), format!("{:?}", s.bounded_loci)),
        ("sync_holding_loci".to_string(), format!("{:?}", s.sync_holding_loci)),
        ("sync_forms".to_string(), format!("{:?}", s.sync_forms)),
        ("carries".to_string(), format!("{:?}", s.carries)),
        ("unbounded_fns".to_string(), format!("{:?}", s.unbounded_fns)),
        ("locus_shapes".to_string(), format!("{:?}", s.locus_shapes)),
        ("fns".to_string(), format!("{:?}", s.fns.keys().collect::<Vec<_>>())),
    ];
    for (k, f) in &s.fns {
        v.push((format!("fn {}", k.display()), format!("{:?}", f)));
    }
    v
}

/// The target's own rows and its program-alone summary, and how many of
/// the program's edges reach into the copy. `None` when the target does
/// not load or its scope is blocked. On a thread of its own: a whole DNA
/// seed's walk is deep.
fn measure(target: &str) -> Option<(Vec<(String, String)>, Vec<(String, String)>, usize)> {
    let path = root().join(target);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config).ok()?;
                let summary = snap.demand_alloc_summary().ok()?;
                let bundle = snap.bundle();
                let alone: Vec<_> = bundle.programs.values().map(|p| (*p, &bundle.snapshot)).collect();
                let alone = summarize_identified(&alone, &bundle.import_renames);
                let into_copy = summary
                    .fns
                    .values()
                    .filter(|f| summary.is_own(&f.key))
                    .flat_map(|f| f.calls.iter())
                    .filter(|c| {
                        matches!(&c.callee, Callee::Resolved(k) if !summary.is_own(k))
                            || c.via_interface.as_ref().is_some_and(|i| summary.analysis_copy_interfaces.contains(i))
                    })
                    .count();
                Some((fields(&summary.own_rows()), fields(&alone), into_copy))
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn the_own_rows_are_the_program_alone_on_every_target() {
    let mut summarized = 0;
    let mut into_copy = 0;
    let mut moved: Vec<String> = Vec::new();
    for t in targets() {
        let Some((own, alone, n)) = measure(&t) else { continue };
        summarized += 1;
        into_copy += n;
        for ((field, a), (_, b)) in own.iter().zip(&alone) {
            if a != b {
                moved.push(format!("{t}: {field}"));
            }
        }
        if own.len() != alone.len() {
            moved.push(format!("{t}: the fn set"));
        }
    }
    assert!(moved.is_empty(), "the own rows differ from the program alone:\n{}", moved.join("\n"));
    assert!(summarized >= 169, "every target with a summary is measured ({summarized})");
    assert!(into_copy > 0, "the program's calls reach into the copy, so the projection is exercised");
}

#[test]
fn the_own_rows_are_the_program_alone_on_every_corpus_program() {
    let mut checked = 0;
    let mut moved: Vec<String> = Vec::new();
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        // The api runtime is never a program of its own: it joins a program
        // that serves a surface, at an offset window the summary marks as
        // the stdlib's (`the_appended_runtime_is_not_the_programs_own`).
        // Alone at offset 0 it is not appended, so no window marks it.
        if p.source == hale_stdlib::API_RUNTIME_SOURCE {
            continue;
        }
        let mut programs = std::collections::BTreeMap::new();
        programs.insert("app.hl".to_string(), &program);
        let bundle = hale_types::symbol::Bundle::new(programs);
        let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
        let alone = summarize_identified(&[(&program, &bundle.snapshot)], &[]);
        let (own, alone) = (fields(&summary.own_rows()), fields(&alone));
        checked += 1;
        if own != alone {
            let field = own.iter().zip(&alone).find(|(a, b)| a != b).map(|(a, _)| a.0.clone());
            moved.push(format!("{}: {}", p.origin, field.unwrap_or_else(|| "the fn set".to_string())));
        }
    }
    assert!(moved.is_empty(), "the own rows differ from the program alone:\n{}", moved.join("\n"));
    assert!(checked > 2000, "the corpus is measured ({checked})");
}

/// `http-hello` starts the TCP `Listener`, so the summary reaches the
/// Listener's hooks; its resource budget counts its own `Listener { }`
/// alone, never the fd-acquiring sites in the copy's bodies, and no fd
/// leak.
#[test]
fn the_resource_budget_counts_the_programs_own_fd_sites() {
    let path = root().join("crates/hale-codegen/tests/fixtures/examples/http-hello");
    let config = Config::check(true, false);
    let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config).ok().expect("http-hello loads");
    let summary = snap.demand_alloc_summary().expect("http-hello has a summary");
    let bundle = snap.bundle();
    let table = snap.demand_placement().expect("http-hello has a placement table");
    let reached = summary.reached.as_ref().expect("the copy is beside the program");
    assert!(
        reached.iter().any(|k| k.locus.as_deref() == Some("__StdIoTcpListener") && !summary.is_own(k)),
        "http-hello starts the Listener, and its hooks are reached"
    );
    let budget = hale_types::resource_budget::budget_for_programs(&bundle, table, summary);
    assert_eq!(budget.fd_open_sites, 1, "the program's own Listener");
    let mut whole = summary.clone();
    whole.analysis_copy.clear();
    whole.analysis_copy_loci.clear();
    whole.analysis_copy_interfaces.clear();
    let read_whole = hale_types::resource_budget::budget_for_programs(&bundle, table, &whole);
    assert_eq!(read_whole.fd_open_sites, 9, "the copy's bodies hold 8 more, which are not the program's");
    assert!(hale_types::resource_budget::resource_leak_diags(summary).is_empty(), "no fd leak");
}

/// `dna/api` starts the HTTP loop and calls through stdlib handles: its
/// summary resolves those calls into the copy, and its own rows hold
/// each as the unresolved call by the method's bare name, the receiver
/// type kept.
#[test]
fn a_call_into_the_copy_is_the_unresolved_call() {
    let checked = measure("dna/api").map(|(own, alone, n)| (own == alone, n));
    let (same, n) = checked.expect("dna/api has a summary");
    assert!(n > 0, "dna/api calls into the copy");
    assert!(same, "dna/api's own rows are its program-alone summary");
}

/// The api runtime a serving program carries is stdlib source appended to
/// it (`rpc_expand`): the program's own rows are its own fns and loci, and
/// none of the runtime's `__StdApi*`. The witness serves surfaces, so the
/// runtime is in its bundle and in the full summary, as the copy's.
#[test]
fn the_appended_runtime_is_not_the_programs_own() {
    let path = root().join("tests/hale/api/witness_test.hl");
    let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("the witness loads");
    let summary = snap.demand_alloc_summary().expect("the witness has a summary");
    let runtime = summary.fns.keys().filter(|k| k.locus.as_deref() == Some("__StdApiExposure")).count();
    assert!(runtime > 0, "the runtime is in the full summary");
    assert!(
        summary.fns.keys().filter(|k| k.locus.as_deref() == Some("__StdApiExposure")).all(|k| !summary.is_own(k)),
        "every runtime fn is the copy's"
    );
    assert!(!summary.is_own_locus("__StdApiExposure"), "the runtime's locus is the copy's");
    let own = summary.own_rows();
    assert!(own.fns.keys().all(|k| !k.locus.as_deref().is_some_and(|l| l.starts_with("__StdApi"))), "no runtime row among the own rows");
    assert!(own.fns.keys().any(|k| k.locus.as_deref() == Some("__RpcSurface_2")), "the program's own surfaces are rows");
}
