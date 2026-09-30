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
    load(file, LoadMode::SeedDirectoryOnly, &Overlay::new(&buffers), Config::editor())
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
    assert_eq!(builds["expression_typing"], 1, "the check itself ran");
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
        // `--dump-model` and a second demand of every family.
        s.demand_model().expect("a clean program has a model");
        s.demand_check().expect("still checked");
        s.demand_scope().expect("still scoped");
        assert_eq!(s.builds()["model"], 1, "the dump reuses the check's model");
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
            for family in ["seed_loading", "desugar_sequence", "snapshot_identity", "top_scope", "expression_typing", "model"] {
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
    s.demand_model().expect("the build's identity reads the model");
    assert_eq!(s.builds()["model"], 1);
    assert_at_most_once(&s, "build");
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
    for family in ["top_scope", "expression_typing", "model", "claims"] {
        assert_eq!(builds[family], 0, "harness: `{family}` was not demanded");
    }
    assert!(s.source_map().is_empty(), "a bare program has no files");
}

/// The key names what was loaded, not only what was asked: the same
/// entry through the whole-seed load and the editor's directory load
/// reads different program sets and gets different keys; editing a
/// file on disk changes the key; two bare programs differ (outside
/// review of #1283, finding 2).
#[test]
fn a_snapshot_key_tells_different_loads_apart() {
    let d = seed("key", NO_CLAIMS);
    std::fs::write(d.join("sibling.hl"), "fn helper() -> Int { return 1; }\n").unwrap();
    let entry = d.join("app.hl");
    let whole = load(&entry, LoadMode::WholeSeed, &Disk, Config::editor());
    let editor_load = load(&entry, LoadMode::SeedDirectoryOnly, &Disk, Config::editor());
    assert_ne!(whole.programs().len(), editor_load.programs().len(), "the modes read different sets");
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
    assert_ne!(ka.key(), kb.key(), "two bare programs, two keys");
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
