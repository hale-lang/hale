//! F.40 phase 3, the entry row's consumers: the corrections each reader
//! of `main` made when it switched to the row, as `hale check` says
//! them. E0 decided the shapes (`check_entry_decisions.rs`): an imported
//! `main locus` is not the entry, a module-nested one is not either, and
//! with several the entry is the last; the placement-safety rules read
//! the row's lowering root, the `main locus` lowering deploys (the
//! first of the seed's own, a module-nested one included). Each test
//! here pins one reader's answer on one of those shapes, beside a
//! control that shows the reading still fires where it should. Section
//! 1 is the checker's readers; section 2 the retired `--api` flag and
//! the roles `--matrix` maps.

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

/// `build --api <path>` injected an `api:` entry into the seed's `main
/// locus`; the entry is retired, and the flag is refused with the
/// replacement, whatever the seed.
#[test]
fn the_api_flag_is_retired() {
    let root = scratch("api_flag");
    let out_bin = root.join("out.bin");
    let app = seed(&root, "app", "main locus App { }\nfn main() { App { }; }\n");
    let out = hale(&[
        "build".as_ref(),
        "--api".as_ref(),
        "/tmp/hale-entry-consumers-api.sock".as_ref(),
        app.as_os_str(),
        "-o".as_ref(),
        out_bin.as_os_str(),
    ]);
    assert!(out.contains("`--api <path>` is retired") && out.contains("api::serve("), "{out}");
    assert!(!out.contains("built:"), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

const SUPPORT_UNMAPPED: &str = "role(s) `support` are not mapped";

/// The roles `--matrix` asks an environment to map are the `role`
/// declarations of the entrypoint's bundle (a served program no longer
/// adds an implicit `owner`): an unmapped one is reported, a mapped one,
/// `[]` included, is not.
#[test]
fn the_matrix_roles_are_the_declared_roles() {
    let manifest = |roles: &str| {
        format!("[claims]\nno_base = true\n\n[environments.dev]\nsource_only = true\nentrypoints = [\"two\"]\n\n[environments.dev.roles]\n{roles}")
    };
    let matrix = |tag: &str, roles: &str| {
        let root = scratch(tag);
        seed(&root, "two", "role support;\nmain locus App { }\nfn main() { App { }; }\n");
        std::fs::write(root.join("hale.toml"), manifest(roles)).unwrap();
        let out = hale(&["check".as_ref(), "--matrix".as_ref(), root.as_os_str()]);
        let _ = std::fs::remove_dir_all(&root);
        out
    };
    let unmapped = matrix("roles_unmapped", "");
    assert!(unmapped.contains(SUPPORT_UNMAPPED), "{unmapped}");
    let mapped = matrix("roles_mapped", "support = []\n");
    assert!(!mapped.contains(SUPPORT_UNMAPPED), "{mapped}");
    assert!(!mapped.contains("owner"), "no implicit `owner` is asked for: {mapped}");
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
