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
    let builds = s.builds();
    assert_eq!(
        builds["model"], 0,
        "a program with no claims, no constitution, and no judged \
         annotation must not derive an ApplicationModel on the LSP's path"
    );
    assert_eq!(builds["claims"], 0);
    assert_eq!(builds["effects"], 0, "a program with no claims runs no effects fixpoint on the LSP's path");
    assert_eq!(builds["expression_typing"], 1, "the check itself ran");
    // Both stages ran (the allocation advisory is the editor's typing
    // stage's), the second with nothing to judge: a program with no law
    // costs no model however the editor publishes the stages.
    assert_eq!((builds["typing_stage"], builds["laws_stage"]), (1, 1), "each stage of the check once");
    // The checker's rules read the snapshot's rows: with no model built,
    // the check is the only demand that builds them, so this count is
    // one only when the checker consumed the snapshot's family. The test
    // entry (`check_bundle_opts_scoped`) builds its own, which no
    // snapshot counts. The bus graph is one of them since rules 7, 9 and
    // 10 read it, the ownership graph since rule 20 does (F.40 phase 3,
    // C4).
    for family in ["handler_routing", "flows", "entrypoint", "bus_graph", "ownership"] {
        assert_eq!(builds[family], 1, "the checker reads the snapshot's `{family}`");
    }
    assert_eq!(builds["alloc_summary"], 1, "the check's certificate engine reads the snapshot's summary");
    assert_at_most_once(&s, "lsp");
    let _ = std::fs::remove_dir_all(&d);
}

const WITH_CODEC: &str = r#"
type Tick { sym: String = ""; price: Int = 0; }
type EncErr { kind: String = ""; }
type DecErr { kind: String = ""; }

topic TickTopic { payload: Tick; subject: "ticks"; }

locus TickJsonCodec {
    PARAMS
    fn encode(v: Tick) -> Bytes fallible(EncErr) {
        MUTATION
        return std::bytes::from_string(v.sym);
    }
    fn decode(b: Bytes) -> Tick fallible(DecErr) {
        return Tick { sym: "x", price: 0 };
    }
}

main locus App {
    bus { publish TickTopic; }
    bindings {
        TickTopic: unix("/ticks.sock") codec(TickJsonCodec { });
    }
}
fn main() { App { }; }
"#;

/// The checker reads the effect rows' purity column on request: a
/// codec binding's purity assertion demands them on the editor's path
/// (the program has no claims), once, and an impure codec is refused
/// through the snapshot as through the bundle entry.
#[test]
fn a_codec_binding_demands_the_effect_rows_once() {
    let pure = WITH_CODEC.replace("PARAMS", "").replace("MUTATION", "");
    let d = seed("codec-pure", &pure);
    let s = editor(&d.join("app.hl"), &pure);
    assert_clean(&s);
    assert_eq!(s.builds()["model"], 0, "no claims, no model");
    assert_eq!(s.builds()["effects"], 1, "the codec's purity assertion read the rows");
    assert_at_most_once(&s, "lsp");
    let _ = std::fs::remove_dir_all(&d);

    let impure = WITH_CODEC
        .replace("PARAMS", "params { calls: Int = 0; }")
        .replace("MUTATION", "self.calls = self.calls + 1;");
    let d = seed("codec-impure", &impure);
    let s = check(&d);
    let checked = s.demand_check().expect("the check runs");
    assert!(
        checked.diags.iter().any(|x| x.message.contains("is not safe to dispatch from arbitrary threads")),
        "an impure codec is refused: {:?}",
        checked.diags.iter().map(|x| &x.message).collect::<Vec<_>>()
    );
    assert_eq!(s.builds()["effects"], 1);
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
        // The allocation summary: one per snapshot, the one the check's
        // certificate engine read and the effect rows walked.
        let summary = s.demand_alloc_summary().expect("a clean program has a summary");
        let rows = s.demand_effects().expect("the rows");
        assert!(std::ptr::eq(summary, &*rows.summary), "the effect rows walk the snapshot's summary");
        assert_eq!(s.builds()["alloc_summary"], 1, "the summary is built once per snapshot");
        // The effects certificate report the law's evidence read is the
        // typing's: `--dump-topology` after the check reads the same
        // report and runs no second check.
        let report = s.demand_effect_certificates().expect("the check's report") as *const _;
        let artifact = s.with_env(|| {
            hale_types::topology::dump_topology_over(
                &s.bundle(),
                s.demand_model().expect("the check's model"),
                s.demand_effect_certificates().expect("the same report"),
                summary,
            )
        });
        assert!(artifact.contains("\"claims\""), "the artifact carries the law");
        assert_eq!(report, s.demand_effect_certificates().expect("still there") as *const _);
        assert_eq!(s.builds()["expression_typing"], 1, "the report is the one typing's");
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
                s.demand_entry().expect("the entry row");
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
                "entrypoint",
                "top_scope",
                "expression_typing",
                "bus_graph",
                "ownership",
                "handler_routing",
                // The check reads it, and the lifecycle plan and lowering
                // the same one (F.40 phase 4, Q1).
                "flows",
                "alloc_summary",
                "effects",
                "model",
                "typing_stage",
                "laws_stage",
                // The check's `bare_fallible` law reads it, and lowering
                // the same one.
                "typed_bodies",
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

/// F.40 phase 3, C5: on a build path the count is the whole truth. The
/// lowering view derives no scope, bus graph or ownership graph of the
/// user's program: its scope is the snapshot's, and its graphs are the
/// snapshot's rows (the stdlib's after them), so the one `builds()`
/// counts for each is the only derivation, lowering's included.
#[test]
fn a_build_derives_the_scope_and_each_graph_once_lowering_included() {
    let d = seed("one-graph", WITH_CLAIM);
    let s = build(&d.join("app.hl"));
    let view = s.demand_lowering().unwrap_or_else(|_| panic!("a clean program is lowered"));
    let builds = s.builds();
    for family in ["top_scope", "bus_graph", "ownership"] {
        assert_eq!(builds[family], 1, "`{family}`: one derivation, and lowering reads it");
    }
    let scope = s.demand_scope().expect("scoped");
    assert_eq!(format!("{:?}", view.top), format!("{scope:?}"), "the view's scope is the snapshot's");
    let bus = s.demand_bus_graph().expect("the graph");
    let ids = |rows: &[hale_types::bus_graph::PublishRow]| rows.iter().map(|r| r.id.0).collect::<Vec<_>>();
    assert_eq!(
        ids(&view.bus.rows.publishes[..bus.rows.publishes.len()]),
        ids(&bus.rows.publishes),
        "the view's bus rows are the snapshot's, first"
    );
    let own = s.demand_ownership_graph().expect("the graph");
    let names = |g: &hale_types::ownership_graph::OwnershipGraph| g.declarations.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&view.ownership)[..own.declarations.len()], names(own)[..], "the view's ownership rows are the snapshot's, first");
    assert_at_most_once(&s, "build");
    let _ = std::fs::remove_dir_all(&d);
}

const WITH_FLOWS: &str = r#"
locus Worker { params { ran: Int = 0; } run() { self.ran = 1; } }
type Job = Worker;
locus Pool {
    params { n: Int = 0; }
    accept(c: Job) { }
    release(c: Job) { self.n = self.n + 1; }
    run() { Worker { }; }
}
locus Manager<T> {
    params { released: Int = 0; }
    accept(c: T) { }
    release(c: T) { self.released = self.released + 1; }
}
fn main() { Pool { }; let a: Manager<Worker> = Manager { }; }
"#;

/// F.40 phase 4, Q1: the flow rows are surveyed once per snapshot, and
/// the lowering view reads that survey. It used to survey the merged
/// program again; lowering reads the rows by locus name (`is_flow`,
/// `specialize`), and the stdlib declares no `release` clause and no
/// type alias, so that survey's clauses are the snapshot's, row for row:
/// an alias followed, a template's clause kept for its specializations.
#[test]
fn a_build_surveys_the_flows_once_and_lowering_reads_that_survey() {
    let std = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    assert!(hale_types::flows::survey(&[std], &[]).is_empty(), "the stdlib declares no `release` clause");
    let declared = hale_types::handler_routing::DeclaredNames::of(&[std]);
    assert!(declared.aliases.is_empty(), "the stdlib declares no type alias a clause's child could follow");

    let d = seed("one-flows", WITH_FLOWS);
    let s = build(&d.join("app.hl"));
    assert_clean(&s);
    let view = s.demand_lowering().unwrap_or_else(|_| panic!("a clean program is lowered"));
    assert_eq!(s.builds()["flows"], 1, "the check, the lifecycle plan and lowering read one survey");
    let rows = |flows: &[hale_types::flows::Flow]| {
        flows
            .iter()
            .flat_map(|f| {
                f.clauses.iter().map(move |c| {
                    (f.child.clone(), c.owner.clone(), c.param.clone(), c.span, c.locus.clone(), c.template.is_some())
                })
            })
            .collect::<Vec<_>>()
    };
    let snapshot = rows(s.demand_flows().expect("the rows"));
    assert_eq!(
        snapshot.iter().map(|r| (r.0.as_str(), r.4.as_deref(), r.5)).collect::<Vec<_>>(),
        vec![("Job", Some("Worker"), false), ("T", None, true)],
        "the alias resolved, the template's clause kept"
    );
    assert_eq!(rows(&view.flows), snapshot, "the view's rows are the snapshot's");
    let merged = hale_types::flows::survey(&[&view.merged], &view.import_renames);
    assert_eq!(rows(&merged), snapshot, "the merged program's survey is the snapshot's, row for row");
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
/// lowered without a check gating it (`Config::harness`). Lowering
/// reads the typed-body table (F.40 phase 3, E4), so the typing runs
/// for it, once, and its diagnostics gate nothing.
#[test]
fn the_harness_snapshot_lowers_without_a_check() {
    let program = hale_syntax::parse_source(NO_CLAIMS).expect("the fixture parses");
    let s = match Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) {
        Ok(s) => s,
        Err(_) => panic!("a bare program's snapshot is not refused"),
    };
    assert!(s.demand_lowering().is_ok(), "the harness lowers what it is handed");
    let builds = s.builds();
    // Lowering reads the form rows (F.40 phase 3, C1), which read the
    // scope: both are demanded once, where the load's sync inference
    // pre-pass used to build a scope of its own outside the counts.
    // And the typed-body table, the typing's record: the typing and the
    // families the checker reads run once for it.
    // The lifecycle plan is carried to the emitters as well.
    for family in [
        "seed_loading",
        "desugar_sequence",
        "snapshot_identity",
        "top_scope",
        "sync_inference",
        "entrypoint",
        "bindings",
        "handler_routing",
        "flows",
        "ownership",
        "bus_graph",
        "alloc_summary",
        "placement",
        "intra_locus",
        "expression_typing",
        "typed_bodies",
        "lowering_view",
        "lifecycle_order",
    ] {
        assert_eq!(builds[family], 1, "harness: `{family}`");
    }
    for family in ["effects", "model", "claims"] {
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
