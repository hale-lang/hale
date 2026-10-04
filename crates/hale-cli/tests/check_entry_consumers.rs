//! F.40 phase 3, the entry row's consumers: the corrections each reader
//! of `main` made when it switched to the row, as `hale check` says
//! them. E0 decided the shapes (`check_entry_decisions.rs`): an imported
//! `main locus` is not the entry, a module-nested one is not either, and
//! with several the entry is the last; the placement-safety rules read
//! the row's lowering root, the `main locus` lowering deploys (the
//! first of the seed's own, a module-nested one included). Each test
//! here pins one reader's answer on one of those shapes, beside a
//! control that shows the reading still fires where it should.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_entry_consumers_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn seed(root: &Path, name: &str, text: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), text).unwrap();
    d
}

fn check(seed: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(seed)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

// ---------------------------------------------------------------- 1 of 4

/// A seed that publishes `Out` and subscribes nothing: rule 9's orphan.
const PUBLISHER: &str = "\
type Msg { v: Int = 0; }

topic Out { payload: Msg; subject: \"consumers.out\"; }

main locus App {
    bus { publish Out; }
    run() { Out <- Msg { v: 1 }; }
}

fn main() { App { }; }
";

const ORPHAN: &str = "bus topic `Out` is published but has no subscriber";

/// The orphan lint is lifted under an api binding (GH #1106), and the
/// binding that binds is the entry's: an imported `main locus`'s `api:`
/// entry is inert (GH #1104 piece 5), so it lifts nothing. Before the
/// entry row, any `main locus` carrying one lifted the lint.
#[test]
fn an_imported_mains_api_entry_does_not_lift_the_orphan_lint() {
    let root = scratch("api_bound");
    seed(
        &root,
        "lib",
        "main locus Head {\n    bindings { api: unix(\"/tmp/hale-entry-consumers-lib.sock\", bound: 8, on_full: refuse); }\n}\n\nfn main() { Head { }; }\n",
    );
    // The control: no api anywhere, the orphan is reported.
    let plain = check(&seed(&root, "plain", PUBLISHER));
    assert!(plain.contains(ORPHAN), "{plain}");
    // The importer's own entry carries no api: the library's is inert.
    let out = check(&seed(&root, "app", &format!("import \"../lib\" as lib;\n\n{PUBLISHER}")));
    assert!(out.contains(ORPHAN), "an imported main's api entry lifts nothing: {out}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A handler that blocks, for a locus placed on an async_io pool.
const BLOCKING_WORKER: &str = "\
type Ev { n: Int; }

locus Worker {
    bus { subscribe \"e\" as on_e of type Ev; }
    fn on_e(e: Ev) {
        println(std::io::stdin::read_line());
    }
}
";

const PLACED_ON_WEB: &str = "\
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = web) where async_io; }
";

const ADVISORY: &str = "`Worker::on_e` is placed on the async_io pool `web`, whose single worker it shares";

/// The placement-implied advisory reads the pools of the `main locus`
/// lowering deploys (the row's lowering root): with two, the first.
/// Before the row, every top-level `main locus`'s placement counted, so
/// the second's pool, which nothing spawns, was advised on.
#[test]
fn the_async_io_advisory_reads_the_deployed_root_only() {
    let root = scratch("implied_two");
    // The control: the deployed main places the pool.
    let first = check(&seed(
        &root,
        "first",
        &format!("{BLOCKING_WORKER}\nmain locus App {{\n{PLACED_ON_WEB}}}\n\nmain locus Other {{ }}\n\nfn main() {{ App {{ }}; }}\n"),
    ));
    assert!(first.contains(ADVISORY), "{first}");
    // The second main places it: lowering deploys the first, which
    // spawns no `web`.
    let second = check(&seed(
        &root,
        "second",
        &format!("{BLOCKING_WORKER}\nmain locus App {{ }}\n\nmain locus Other {{\n{PLACED_ON_WEB}}}\n\nfn main() {{ App {{ }}; }}\n"),
    ));
    assert!(second.contains("more than one `main` locus declared"), "{second}");
    assert!(!second.contains(ADVISORY), "the second main's pool is not spawned: {second}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A module-nested `main locus` is not the entry, but lowering deploys
/// it and spawns its pools, so the advisory reads its placement (the
/// placement-safety side of E0's decision 2). Before the row, only a
/// top-level `main locus` was read, and the nested one's pool went
/// unadvised.
#[test]
fn the_async_io_advisory_reads_a_module_nested_root() {
    let root = scratch("implied_nested");
    let body: String =
        format!("main locus App {{\n{PLACED_ON_WEB}}}\n").lines().map(|l| format!("    {l}\n")).collect();
    let out = check(&seed(&root, "nested", &format!("{BLOCKING_WORKER}\nmodule inner {{\n{body}}}\n\nfn main() {{ App {{ }}; }}\n")));
    assert!(out.contains(ADVISORY), "the deployed nested main's pool is advised on: {out}");
    let _ = std::fs::remove_dir_all(&root);
}
