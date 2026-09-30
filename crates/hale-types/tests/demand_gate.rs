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
//!   the check.
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
use hale_frontend::snapshot::{Config, Snapshot};
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
            ("lsp", editor(&d.join("app.hl"), text)),
            ("check <file>", check(&d.join("app.hl"))),
            ("check <dir>", check(&d)),
        ];
        for (consumer, s) in &consumers {
            for _ in 0..2 {
                s.demand_scope().expect("scoped");
                s.demand_check().expect("checked");
                s.demand_model().expect("a clean program has a model");
                let _ = s.bundle();
            }
            let builds = s.builds();
            for family in ["seed_loading", "desugar_sequence", "snapshot_identity", "top_scope", "expression_typing", "model"] {
                assert_eq!(builds[family], 1, "{consumer}: `{family}`");
            }
            assert_at_most_once(s, consumer);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
