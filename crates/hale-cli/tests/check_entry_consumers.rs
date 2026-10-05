//! F.40 phase 3, the entry row's consumers: the corrections each reader
//! of `main` made when it switched to the row, as `hale check` says
//! them. E0 decided the shapes (`check_entry_decisions.rs`): an imported
//! `main locus` is not the entry, a module-nested one is not either, and
//! with several the entry is the last; the placement-safety rules read
//! the row's lowering root, the `main locus` lowering deploys (the
//! first of the seed's own, a module-nested one included). Each test
//! here pins one reader's answer on one of those shapes, beside a
//! control that shows the reading still fires where it should. Section
//! 1 is the checker's readers; section 2 the api binding, `--api` and
//! the roles `--matrix` maps, which read the lowering root: the `main
//! locus` the binding joins.

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
/// lowering deploys (the row's root, the entry since L4): with two
/// (rule 1's error), the last, as the row takes the entry. Before the
/// row, every top-level `main locus`'s placement counted, so the other
/// one's pool, which nothing spawns, was advised on.
#[test]
fn the_async_io_advisory_reads_the_deployed_root_only() {
    let root = scratch("implied_two");
    // The control: the deployed main places the pool.
    let deployed = check(&seed(
        &root,
        "deployed",
        &format!("{BLOCKING_WORKER}\nmain locus Other {{ }}\n\nmain locus App {{\n{PLACED_ON_WEB}}}\n\nfn main() {{ App {{ }}; }}\n"),
    ));
    assert!(deployed.contains(ADVISORY), "{deployed}");
    // The other main places it: lowering deploys the entry, which
    // spawns no `web`.
    let other = check(&seed(
        &root,
        "other",
        &format!("{BLOCKING_WORKER}\nmain locus Other {{\n{PLACED_ON_WEB}}}\n\nmain locus App {{ }}\n\nfn main() {{ App {{ }}; }}\n"),
    ));
    assert!(other.contains("more than one `main` locus declared"), "{other}");
    assert!(!other.contains(ADVISORY), "the other main's pool is not spawned: {other}");
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

// ---------------------------------------------------------------- 2 of 4

fn hale(args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// `--api` puts its entry on the `main locus` the binding joins, the
/// seed's own; an imported library's bindings are inert, so a seed whose
/// only `main` is imported has nowhere to put it and is refused, saying
/// why. Before the row, the entry went on the library's `main locus`,
/// no binding was generated from it, and the build succeeded with no
/// api.
#[test]
fn api_refuses_a_seed_whose_only_main_is_imported() {
    let root = scratch("api_flag");
    seed(&root, "lib", "main locus Head { }\nfn main() { Head { }; }\n");
    let out_bin = root.join("out.bin");
    let build = |dir: &Path| {
        hale(&[
            "build".as_ref(),
            "--api".as_ref(),
            "/tmp/hale-entry-consumers-api.sock".as_ref(),
            dir.as_os_str(),
            "-o".as_ref(),
            out_bin.as_os_str(),
        ])
    };
    // The control: a bare `fn main` is refused as before.
    let bare = build(&seed(&root, "bare", "fn main() { println(\"hi\"); }\n"));
    assert!(bare.contains("--api needs a `main locus` to bind"), "{bare}");
    let out = build(&seed(&root, "app", "import \"../lib\" as lib;\nfn main() { println(\"hi\"); }\n"));
    assert!(
        out.contains(
            "--api needs the seed's own `main locus` to bind: the api entry lives in its `bindings { }` block, \
             and the only `main locus` here is an imported library's, whose bindings are inert"
        ),
        "{out}"
    );
    assert!(!out.contains("built:"), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

const OWNER_UNMAPPED: &str = "role(s) `owner` are not mapped";

/// `owner` joins the roles `--matrix` asks an environment to map when
/// the program has an api binding, and the binding is the one generated
/// into the `main locus` lowering deploys (the row's root, the entry
/// since L4): with two (rule 1's error), the last, as the row takes the
/// entry, and the other one's `api:` entry binds nothing and gates
/// nothing. Before the row, any `main locus` of the seed's own carrying
/// one declared `owner`.
#[test]
fn the_matrix_roles_read_the_deployed_roots_binding() {
    let api = "    bindings { api: unix(\"/tmp/hale-entry-consumers-roles.sock\", bound: 8, on_full: refuse); }\n";
    let manifest =
        "[claims]\nno_base = true\n\n[environments.dev]\nsource_only = true\nentrypoints = [\"two\"]\n\n[environments.dev.roles]\n";
    let matrix = |tag: &str, program: String| {
        let root = scratch(tag);
        seed(&root, "two", &program);
        std::fs::write(root.join("hale.toml"), manifest).unwrap();
        let out = hale(&["check".as_ref(), "--matrix".as_ref(), root.as_os_str()]);
        let _ = std::fs::remove_dir_all(&root);
        out
    };
    // The control: the deployed root carries the entry.
    let deployed = matrix("roles_deployed", format!("main locus Other {{ }}\nmain locus App {{\n{api}}}\nfn main() {{ App {{ }}; }}\n"));
    assert!(deployed.contains(OWNER_UNMAPPED), "{deployed}");
    let other = matrix("roles_other", format!("main locus Other {{\n{api}}}\nmain locus App {{ }}\nfn main() {{ App {{ }}; }}\n"));
    assert!(other.contains("more than one `main` locus declared"), "{other}");
    assert!(!other.contains(OWNER_UNMAPPED), "the other main's entry binds nothing: {other}");
}

// ---------------------------------------------------------------- 3 of 4

/// The model's `entrypoint` names the root its arrangement is rooted at,
/// the placement table's (the row's lowering root): the seed's own `main
/// locus`, never an imported library's, which lowering does not deploy;
/// with none, `main`. Before the row it named the first `main locus` of
/// the merged program, so a seed whose only `main` is imported named the
/// library's.
#[test]
fn the_models_entrypoint_is_the_deployed_root() {
    let root = scratch("model_entry");
    seed(&root, "alib", "main locus Head { }\nfn main() { Head { }; }\n");
    let dump = |dir: &Path| hale(&["check".as_ref(), "--dump-model".as_ref(), dir.as_os_str()]);
    // The controls: no import, and the seed's own main beside an
    // imported one.
    let plain = dump(&seed(&root, "plain", "main locus App { }\nfn main() { App { }; }\n"));
    assert!(plain.contains("\nentrypoint App\n"), "{plain}");
    let own = dump(&seed(&root, "own", "import \"../alib\" as lib;\nmain locus Own { }\nfn main() { Own { }; }\n"));
    assert!(own.contains("\nentrypoint Own\n"), "{own}");
    let bare = dump(&seed(&root, "bare", "import \"../alib\" as lib;\nfn main() { }\n"));
    assert!(bare.contains("\nentrypoint main\n"), "an imported main is deployed by nothing here: {bare}");
    let _ = std::fs::remove_dir_all(&root);
}
