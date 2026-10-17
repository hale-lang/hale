//! The check's allocation summary reads the stdlib's analysis copy where
//! the program reaches it (`derive_alloc_summary_reaching`): the copy's
//! bodies outside the reach are not summarized, and the passes that join
//! over the rows read only the rows that run (`AllocSummary::runs`).
//!
//! - A finding whose cause is code the program reaches stands, a method
//!   call whose receiver the walk cannot type reaching the method of its
//!   name on every copy locus reached code starts.
//! - A finding caused only by copy bodies the program never reaches is
//!   gone.
//! - Every row the program reaches, and its effect row, is the row the
//!   whole copy beside it gives.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::alloc_summary::{derive_alloc_summary_over, AllocSummary, FnKey, FnSummary};
use hale_types::effect_rows::derive_effect_rows;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `f` over the snapshot of `path`, loaded as `hale check` loads it, on a
/// thread of its own (a whole seed's walk is deep).
fn over<T: Send>(path: &Path, f: impl FnOnce(&Snapshot) -> T + Send) -> T {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let snap = Snapshot::load(path, LoadMode::WholeSeed, &Disk, config).ok().expect("loads");
                f(&snap)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// A program written to a file of its own, checked: the program's own
/// leak sites, by owner.
fn own_leaks(name: &str, src: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("hale-reach-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.hl");
    std::fs::write(&file, src).unwrap();
    let leaks = over(&file, |snap| {
        let summary = snap.demand_alloc_summary().expect("summarized");
        summary.leak_sites().iter().filter(|l| summary.is_own(&l.owner)).map(|l| l.owner.display()).collect()
    });
    let _ = std::fs::remove_dir_all(&dir);
    leaks
}

/// The programs below are plain strings, not raw ones, so the shared
/// corpus does not harvest them: that corpus holds programs whose own
/// rows the copy beside them leaves alone (`alloc_summary_own_rows`), and
/// in these the copy's `set` moves the program's.
///
/// A generic fn calls `set` on a value whose type the walk cannot see:
/// it may be any started locus's `set`, the copy's `std::metrics::Gauge`
/// among them, which the program builds through its registry's `gauge`
/// (a handle's member, sugar for `std::metrics::gauge`) and hands it as a
/// `Settable`. The gauge's `set` runs, and its call of `set` with what it
/// was passed makes every `set` carry anything, the program's own: the
/// program's `Acc::set` keeps its finding.
#[test]
fn an_untyped_receiver_reaches_the_method_on_every_started_copy_locus() {
    let src = concat!(
        "type Note {\n",
        "    text: String;\n",
        "}\n",
        "\n",
        "interface Settable {\n",
        "    fn set(v: Float);\n",
        "}\n",
        "\n",
        "fn poke<T>(g: T) {\n",
        "    g.set(2.0);\n",
        "}\n",
        "\n",
        "locus Acc {\n",
        "    params {\n",
        "        text: String = \"\";\n",
        "        depth: Settable;\n",
        "    }\n",
        "    bus {\n",
        "        subscribe \"demo.note\" as on_note of type Note;\n",
        "    }\n",
        "\n",
        "    fn on_note(n: Note) {\n",
        "        poke(self.depth);\n",
        "        self.set(n.text);\n",
        "    }\n",
        "\n",
        "    fn set(s: String) {\n",
        "        self.text = s + \"!\";\n",
        "    }\n",
        "}\n",
        "\n",
        "fn main() {\n",
        "    let reg = std::metrics::Registry { };\n",
        "    Acc { depth: reg.gauge(\"depth\", std::metrics::Labels { kv: \"\" }) };\n",
        "}\n",
    );
    assert_eq!(own_leaks("untyped", src), ["Acc::set"]);
}

/// The gauge built by a call of the stdlib's factory by its public path,
/// in `main` or in a param's default: the factory runs, so the locus it
/// builds is started and its `set` runs.
#[test]
fn a_factory_called_by_its_public_path_starts_what_it_builds() {
    let in_main = concat!(
        "type Note {\n",
        "    text: String;\n",
        "}\n",
        "\n",
        "interface Settable {\n",
        "    fn set(v: Float);\n",
        "}\n",
        "\n",
        "fn poke<T>(g: T) {\n",
        "    g.set(2.0);\n",
        "}\n",
        "\n",
        "locus Acc {\n",
        "    params {\n",
        "        text: String = \"\";\n",
        "        depth: Settable;\n",
        "    }\n",
        "    bus {\n",
        "        subscribe \"demo.note\" as on_note of type Note;\n",
        "    }\n",
        "\n",
        "    fn on_note(n: Note) {\n",
        "        poke(self.depth);\n",
        "        self.set(n.text);\n",
        "    }\n",
        "\n",
        "    fn set(s: String) {\n",
        "        self.text = s + \"!\";\n",
        "    }\n",
        "}\n",
        "\n",
        "fn main() {\n",
        "    let reg = std::metrics::Registry { };\n",
        "    Acc { depth: std::metrics::gauge(reg, \"depth\", std::metrics::Labels { kv: \"\" }) };\n",
        "}\n",
    );
    assert_eq!(own_leaks("factory-main", in_main), ["Acc::set"]);
    let in_default = concat!(
        "type Note {\n",
        "    text: String;\n",
        "}\n",
        "\n",
        "interface Settable {\n",
        "    fn set(v: Float);\n",
        "}\n",
        "\n",
        "fn poke<T>(g: T) {\n",
        "    g.set(2.0);\n",
        "}\n",
        "\n",
        "locus Acc {\n",
        "    params {\n",
        "        text: String = \"\";\n",
        "        depth: Settable = std::metrics::gauge(std::metrics::Registry { }, \"depth\", std::metrics::Labels { kv: \"\" });\n",
        "    }\n",
        "    bus {\n",
        "        subscribe \"demo.note\" as on_note of type Note;\n",
        "    }\n",
        "\n",
        "    fn on_note(n: Note) {\n",
        "        poke(self.depth);\n",
        "        self.set(n.text);\n",
        "    }\n",
        "\n",
        "    fn set(s: String) {\n",
        "        self.text = s + \"!\";\n",
        "    }\n",
        "}\n",
        "\n",
        "fn main() {\n",
        "    Acc { };\n",
        "}\n",
    );
    assert_eq!(own_leaks("factory-default", in_default), ["Acc::set"]);
}

/// The program's `set`, called from a bus handler, stores what it was
/// passed. Only copy bodies the program never reaches call a `set` on a
/// receiver the walk cannot type with a value that may carry anything,
/// so nothing that runs makes the parameter carry the field: the store
/// retires what it replaces, and the site is no finding. Beside the whole
/// copy, every body counted, it was one.
#[test]
fn a_finding_only_unreached_copy_bodies_cause_is_gone() {
    let src = r#"
type Note {
    text: String;
}

locus Book {
    params {
        text: String = "";
    }
    bus {
        subscribe "demo.note" as on_note of type Note;
    }

    fn on_note(n: Note) {
        self.set(n.text);
    }

    fn set(s: String) {
        self.text = s + "!";
    }
}

fn main() {
    Book { };
}
"#;
    assert_eq!(own_leaks("unreached", src), Vec::<String>::new());
}

/// `f`'s calls with each dispatch group renumbered in order of first use
/// over the rows `keys` names: the groups only tie a row's alternatives
/// together, and the ids shift with the rows a summary holds.
fn canonical(s: &AllocSummary, keys: &[&FnKey]) -> BTreeMap<FnKey, String> {
    let mut ids: BTreeMap<u32, u32> = BTreeMap::new();
    keys.iter()
        .map(|k| {
            let mut f: FnSummary = s.fns[*k].clone();
            for c in f.calls.iter_mut() {
                if let Some(g) = c.dispatch_group {
                    let n = ids.len() as u32;
                    c.dispatch_group = Some(*ids.entry(g).or_insert(n));
                }
            }
            ((*k).clone(), format!("{f:?}"))
        })
        .collect()
}

/// Every row the check's summary reaches, and its effect row, is the
/// whole copy's (the summary beside the whole copy, over the snapshot's
/// whole owned-view column).
#[test]
fn every_reached_row_is_the_whole_copys() {
    for target in [
        "crates/hale-codegen/tests/fixtures/examples/hello-world",
        "crates/hale-codegen/tests/fixtures/examples/92-build-an-api",
        "dna/host",
    ] {
        over(&root().join(target), |snap| {
            let reaching = snap.demand_alloc_summary().expect("summarized");
            let whole = derive_alloc_summary_over(&snap.bundle(), snap.demand_owned_views().expect("viewed"));
            let reached = reaching.reached.as_ref().expect("the copy is beside the program");
            assert_eq!(whole.reached.as_ref(), Some(reached), "{target}: the rows that run");
            let copy = |k: &FnKey| k.decl.is_some_and(|d| d.universe == hale_types::placement::SiteUniverse::StdlibAnalysis);
            let rowless: Vec<String> =
                reached.iter().filter(|k| copy(k) && !reaching.fns.contains_key(*k)).map(FnKey::display).collect();
            assert!(rowless.is_empty(), "{target}: reached copy fns with no row: {rowless:?}");
            let keys: Vec<&FnKey> = reached.iter().filter(|k| reaching.fns.contains_key(*k)).collect();
            assert!(keys.iter().all(|k| whole.fns.contains_key(*k)), "{target}");
            let (r, w) = (canonical(reaching, &keys), canonical(&whole, &keys));
            for k in &keys {
                assert_eq!(r[*k], w[*k], "{target}: {}", k.display());
            }
            let top = snap.demand_scope().expect("the scope");
            let rows_r = derive_effect_rows(&snap.bundle(), top, Arc::new(reaching.clone()));
            let rows_w = derive_effect_rows(&snap.bundle(), top, Arc::new(whole.clone()));
            for k in &keys {
                let (a, b) = (&rows_r.rows[*k], &rows_w.rows[*k]);
                assert_eq!(
                    (a.effects, a.known, a.unknown, a.direct, &a.targets, &a.unresolved),
                    (b.effects, b.known, b.unknown, b.direct, &b.targets, &b.unresolved),
                    "{target}: {}'s effect row",
                    k.display()
                );
            }
        });
    }
}
