//! GH #476 acceptance criterion 1 (#515), as per-family accounting
//! (F.40 phase 2.2a): *"One demand-gated model derivation entry point;
//! no-claims LSP path **provably** skips it."*
//!
//! The consumers demand their families from a
//! `hale_frontend::snapshot::Snapshot`, which counts each family's
//! producer runs (`Snapshot::builds`). The gate reads that count for
//! each consumer switched to the snapshot, loaded exactly as the
//! consumer loads it:
//!
//! - the LSP's diagnostics path (`check_and_publish`): the editor's
//!   load over its buffers, `Config::editor()`;
//! - `hale check` (`run_check_impl_labelled`): the whole seed from the
//!   disk, `Config::check`, and `--dump-model` demanding the model after
//!   the check;
//! - every build path (`build`, `run`, `test`, `replay`, `bench`, F.40
//!   phase 2.2b): the whole seed with a build's config, the check, the
//!   lowering view, and the model the build's identity reads;
//! - the test harness (codegen's `build_executable_with_options`): a
//!   bare program's snapshot (`Snapshot::from_program`), whose lowering
//!   is not gated on the check.
//!
//! A program that swears to nothing builds no model on the editor path;
//! one that declares a law builds exactly one on `hale check`, which a
//! later dump reuses; and no family runs twice for one snapshot. A
//! refactor that hoisted the derivation above the claim-surface gate,
//! or a family that rebuilt its prerequisite instead of demanding it,
//! fails here. The counts are the snapshot's own, so the tests of this
//! binary run in parallel without sharing a counter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::{Disk, Overlay, SourceProvider};

const NO_CLAIMS: &str = r#"
type Order { id: Int = 0; }
topic Placed { payload: Order; }

locus Desk {
    params { seen: Int = 0; }
    bus { subscribe Placed as on_placed; }
    fn on_placed(o: Order) { self.seen = o.id; }
}

locus Feed {
    bus { publish Placed; }
    fn go() { Placed <- Order { id: 1 }; }
}

main locus App {
    params { d: Desk = Desk { }; f: Feed = Feed { }; }
}
fn main() { App { }; }
"#;

const WITH_CLAIM: &str = r#"
type Order { id: Int = 0; }
topic Placed { payload: Order; }

locus Desk {
    params { seen: Int = 0; }
    bus { subscribe Placed as on_placed; }
    fn on_placed(o: Order) { self.seen = o.id; }
}

locus Feed {
    bus { publish Placed; }
    fn go() { Placed <- Order { id: 1 }; }
}

main locus App {
    params { d: Desk = Desk { }; f: Feed = Feed { }; }
    claims { one_writer: count publishers(topic Placed) == 1; }
}
fn main() { App { }; }
"#;

/// A scratch seed of this test's own holding `app.hl`: the pid keeps
/// two runs apart, the name keeps two tests of one run apart.
fn seed(name: &str, text: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale-demand-gate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let d = d.canonicalize().unwrap();
    std::fs::write(d.join("app.hl"), text).unwrap();
    d
}

fn load(entry: &Path, mode: LoadMode, src: &dyn SourceProvider, config: Config) -> Snapshot {
    match Snapshot::load(entry, mode, src, config) {
        Ok(s) => s,
        Err(_) => panic!("{} does not load", entry.display()),
    }
}

/// The LSP's load: the file's seed, over the editor's buffer of it.
fn editor(file: &Path, text: &str) -> Snapshot {
    let mut buffers = BTreeMap::new();
    buffers.insert(file.to_path_buf(), text.to_string());
    load(file, LoadMode::Editor, &Overlay::new(&buffers), Config::editor())
}

/// `hale check <target>`'s load.
fn check(target: &Path) -> Snapshot {
    load(target, LoadMode::WholeSeed, &Disk, Config::check(target.is_dir(), false))
}

/// A build path's load (`hale build <target>` for the host, no flags).
fn build(target: &Path) -> Snapshot {
    load(target, LoadMode::WholeSeed, &Disk, Config::build(Target::host()))
}

/// A fixture must be CLEAN, or the check may stop before the gate
/// and the count proves nothing.
fn assert_clean(s: &Snapshot) {
    let checked = s.demand_check().expect("the fixture's check is not blocked");
    let errors: Vec<&String> =
        checked.diags.iter().filter(|d| d.is_error()).map(|d| &d.message).collect();
    assert!(errors.is_empty(), "the fixture must be a clean program: {errors:?}");
}

/// Contract 1: whatever was demanded, no family ran twice.
fn assert_at_most_once(s: &Snapshot, consumer: &str) {
    for (family, n) in s.builds() {
        assert!(n <= 1, "{consumer}: `{family}` ran {n} times for one snapshot");
    }
}

/// The editor path: a program that swears to nothing must not build a
/// model — every keystroke runs this path.
#[test]
fn the_editor_path_builds_no_model_for_a_program_with_no_claims() {
    let d = seed("editor", NO_CLAIMS);
    let s = editor(&d.join("app.hl"), NO_CLAIMS);
    assert_clean(&s);
    // What `check_and_publish` reads beside the check.
    let _ = hale_types::unbounded_alloc_warnings(&s.bundle(), true);
    let builds = s.builds();
    assert_eq!(
        builds["model"], 0,
        "a program with no claims, no constitution, and no judged \
         annotation must not derive an ApplicationModel on the LSP's path"
    );
    assert_eq!(builds["claims"], 0);
    assert_eq!(builds["effects"], 0, "a program with no claims runs no effects fixpoint on the LSP's path");
    for family in ["bus_graph", "ownership"] {
        assert_eq!(builds[family], 0, "the model's input `{family}` is demanded with it");
    }
    assert_eq!(builds["expression_typing"], 1, "the check itself ran");
    // The checker's rules read the snapshot's rows: with no model built,
    // the check is the only demand that builds them, so this count is
    // one only when the checker consumed the snapshot's family. The test
    // entry (`check_bundle_opts_scoped`) builds its own, which no
    // snapshot counts.
    for family in ["handler_routing"] {
        assert_eq!(builds[family], 1, "the checker reads the snapshot's `{family}`");
    }
    assert_at_most_once(&s, "lsp");
    let _ = std::fs::remove_dir_all(&d);
}

/// The other half: the gate must open. `hale check` of a program that
/// declares a law builds the model once, judges over it, and a
/// `--dump-model` after the check reuses it.
#[test]
fn hale_check_of_a_program_with_claims_builds_the_model_once() {
    let d = seed("claims", WITH_CLAIM);
    for target in [d.join("app.hl"), d.clone()] {
        let s = check(&target);
        assert_clean(&s);
        assert_eq!(s.builds()["model"], 1, "a claim is judged over the model");
        assert_eq!(s.builds()["claims"], 1);
        // The checker and the model read the snapshot's graphs: built
        // once, for both. No rule of the check builds its own rows
        // beside the family, so the count is the number of builds.
        for family in ["top_scope", "bus_graph", "ownership", "handler_routing"] {
            assert_eq!(s.builds()[family], 1, "the check and the model demand `{family}`");
        }
        assert_eq!(s.builds()["effects"], 1, "the model reads the effect rows: one fixpoint for the check with a law");
        s.demand_bus_graph().expect("the graph the model read");
        s.demand_ownership_graph().expect("the graph the model read");
        s.demand_handlers().expect("the rows the model read");
        // `--dump-model` and a second demand of every family.
        s.demand_model().expect("a clean program has a model");
        s.demand_check().expect("still checked");
        s.demand_scope().expect("still scoped");
        assert_eq!(s.builds()["model"], 1, "the dump reuses the check's model");
        // The effect rows: one fixpoint, however often demanded.
        let first = s.demand_effects().expect("a clean program has effect rows") as *const _;
        let again = s.demand_effects().expect("still there") as *const _;
        assert_eq!(first, again, "the second demand reads the first result");
        assert_eq!(s.builds()["effects"], 1, "the effects fixpoint runs once per snapshot");
        assert_at_most_once(&s, "check");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Contract 1 across both consumers and both programs: the scope and
/// the typing run exactly once however often they are demanded, the
/// model at most once, the load's own families once.
#[test]
fn every_family_runs_at_most_once_per_snapshot_on_every_switched_consumer() {
    for (name, text) in [("once-plain", NO_CLAIMS), ("once-law", WITH_CLAIM)] {
        let d = seed(name, text);
        let consumers = [
            ("lsp", editor(&d.join("app.hl"), text), false),
            ("check <file>", check(&d.join("app.hl")), false),
            ("check <dir>", check(&d), false),
            ("build <file>", build(&d.join("app.hl")), true),
            ("build <dir>", build(&d), true),
        ];
        for (consumer, s, lowers) in &consumers {
            for _ in 0..2 {
                s.demand_scope().expect("scoped");
                s.demand_check().expect("checked");
                if *lowers {
                    assert!(s.demand_lowering().is_ok(), "{consumer}: a clean program is lowered");
                }
                s.demand_model().expect("a clean program has a model");
                let _ = s.bundle();
            }
            let builds = s.builds();
            for family in [
                "seed_loading",
                "desugar_sequence",
                "snapshot_identity",
                "top_scope",
                "expression_typing",
                "bus_graph",
                "ownership",
                "handler_routing",
                "effects",
                "model",
            ] {
                assert_eq!(builds[family], 1, "{consumer}: `{family}`");
            }
            assert_eq!(builds["lowering_view"], u32::from(*lowers), "{consumer}: `lowering_view`");
            assert_at_most_once(s, consumer);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// The build path's order: the lowering view waits for the check, and a
/// program that swears to nothing lowers without a model until the
/// build's identity asks for one.
#[test]
fn a_build_lowers_after_its_check_and_builds_no_model_it_was_not_asked_for() {
    let d = seed("build-order", NO_CLAIMS);
    let s = build(&d.join("app.hl"));
    assert!(s.demand_lowering().is_ok(), "a clean program is lowered");
    let builds = s.builds();
    assert_eq!(builds["expression_typing"], 1, "the check ran before lowering");
    assert_eq!(builds["lowering_view"], 1);
    assert_eq!(builds["model"], 0, "nothing asked for the model yet");
    assert_eq!(builds["effects"], 0, "nor for the effect rows it reads");
    s.demand_model().expect("the build's identity reads the model");
    assert_eq!(s.builds()["model"], 1);
    assert_eq!(s.builds()["effects"], 1);
    assert_at_most_once(&s, "build");
    let _ = std::fs::remove_dir_all(&d);
}

/// F.40 phase 2, use-site identity: which declaration each use names is
/// resolved once per snapshot, by its mint — the load's, and the
/// lowering view's over the merged program — and nothing the check, the
/// build rules, the model or lowering reads resolves it again. The
/// count is this thread's, so the tests of this binary do not share it.
/// The bundled stdlib's analysis copy has identities of its own, minted
/// once per process (`stdlib_bodies::identities`); it is warmed first,
/// outside the count.
#[test]
fn each_snapshot_resolves_its_uses_once() {
    let _ = hale_types::stdlib_bodies::identities();
    let d = seed("uses-once", WITH_CLAIM);
    let resolved = hale_types::snapshot::resolutions_on_this_thread;
    let before = resolved();
    let s = build(&d.join("app.hl"));
    assert_eq!(resolved() - before, 1, "the load's mint resolves the bundle's uses");
    assert!(!s.bundle().snapshot.binding_of.is_empty(), "`o.id` names the handler's parameter");
    assert_clean(&s);
    s.demand_model().expect("the model");
    assert!(s.demand_lowering().is_ok(), "a clean program is lowered");
    assert_eq!(resolved() - before, 2, "and the lowering view's mint, the merged program's");
    assert_clean(&s);
    assert!(s.demand_lowering().is_ok());
    assert_eq!(resolved() - before, 2, "a second demand resolves nothing");
    let _ = std::fs::remove_dir_all(&d);
}

/// The harness's snapshot: a bare program, shaped by the one load and
/// lowered without a check (`Config::harness`).
#[test]
fn the_harness_snapshot_lowers_without_a_check() {
    let program = hale_syntax::parse_source(NO_CLAIMS).expect("the fixture parses");
    let s = match Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) {
        Ok(s) => s,
        Err(_) => panic!("a bare program's snapshot is not refused"),
    };
    assert!(s.demand_lowering().is_ok(), "the harness lowers what it is handed");
    let builds = s.builds();
    for family in ["seed_loading", "desugar_sequence", "snapshot_identity", "lowering_view"] {
        assert_eq!(builds[family], 1, "harness: `{family}`");
    }
    for family in ["top_scope", "expression_typing", "bus_graph", "ownership", "handler_routing", "effects", "model", "claims"] {
        assert_eq!(builds[family], 0, "harness: `{family}` was not demanded");
    }
    assert!(s.source_map().is_empty(), "a bare program has no files");
}

/// The key names what was loaded, not only what was asked: the same
/// file through the whole-seed load (a seed of one) and the editor's
/// load (its directory) reads different file sets and gets different
/// keys; editing a file on disk changes the key; two bare programs
/// differ (outside review of #1283, finding 2).
#[test]
fn a_snapshot_key_tells_different_loads_apart() {
    let d = seed("key", NO_CLAIMS);
    std::fs::write(d.join("sibling.hl"), "fn helper() -> Int { return 1; }\n").unwrap();
    let entry = d.join("app.hl");
    let whole = load(&entry, LoadMode::WholeSeed, &Disk, Config::editor());
    let editor_load = load(&entry, LoadMode::Editor, &Disk, Config::editor());
    assert_ne!(whole.sources().len(), editor_load.sources().len(), "the modes read different sets");
    assert_ne!(whole.key(), editor_load.key(), "different loads, different keys");
    let again = load(&entry, LoadMode::WholeSeed, &Disk, Config::editor());
    assert_eq!(whole.key(), again.key(), "the same load, the same key");
    std::fs::write(&entry, format!("{NO_CLAIMS}\n// edited\n")).unwrap();
    let edited = load(&entry, LoadMode::WholeSeed, &Disk, Config::editor());
    assert_ne!(whole.key(), edited.key(), "an edit on disk is a different snapshot");
    let a = hale_syntax::parse_source("fn main() { }").unwrap();
    let b = hale_syntax::parse_source("fn main() { let x = 1; }").unwrap();
    let bare = |p| match Snapshot::from_program(p, Vec::new(), Config::build(Target::host())) {
        Ok(s) => s,
        Err(_) => panic!("a bare program shapes"),
    };
    let ka = bare(a);
    let kb = bare(b);
    assert_ne!(ka.key(), kb.key(), "two bare programs, two keys: each handoff is its own load");
}

/// The environment a snapshot's claims are checked for is the
/// snapshot's own: loading a second snapshot for another environment
/// afterwards changes nothing about the first's diagnostics or its
/// artifact's label (outside review of #1283, finding 1).
#[test]
fn a_demand_reads_its_own_snapshots_environment_not_the_last_loaded() {
    use hale_frontend::snapshot::Environment;
    let d = seed("env", NO_CLAIMS);
    let mut dev = Config::build(Target::host());
    dev.environment = Some(Environment { name: "dev".into(), adopt: vec!["Missing".into()] });
    let mut prod = Config::build(Target::host());
    prod.environment = Some(Environment { name: "prod".into(), adopt: Vec::new() });
    let first = load(&d, LoadMode::WholeSeed, &Disk, dev);
    let _second = load(&d, LoadMode::WholeSeed, &Disk, prod);
    let checked = first.demand_check().expect("a claims error does not block the check");
    let msgs: Vec<&String> = checked.diags.iter().map(|d| &d.message).collect();
    assert!(
        msgs.iter().any(|m| m.contains("unknown constitution `Missing`") && m.contains("`[environments.dev]` in hale.toml requires it")),
        "the first snapshot's claims are explained by ITS environment: {msgs:?}"
    );
    let artifact = first.with_env(|| hale_types::topology::dump_topology(&first.bundle()));
    assert!(artifact.contains("\"environment\": \"dev\""), "the artifact carries the first snapshot's label: {}", &artifact[..artifact.len().min(400)]);
}
