//! GH #1951 item 6 (F.51): an import list brings named declarations of
//! the imported seed in bare, and re-exports them through the importing
//! seed's alias as the same declaration.
//!
//! `import "../store" as st { ModelRow, describe as show };` makes
//! `ModelRow` and `show` bare in every file of the importing seed, and
//! an importer of that seed reaches them as `api::ModelRow`, one mangled
//! symbol with `st::ModelRow`. These build and run the seeds on disk as
//! a verb loads them; the refusals (an unknown item, a collision, a file
//! of the seed itself) are `hale-cli`'s `import_lists.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::Snapshot;
use hale_frontend::source::Disk;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Write `files` (path under the scratch dir, text) and return the dir.
fn seeds(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = harness::unique_dir(name);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, text).expect("write seed file");
    }
    dir
}

/// Build the seed `entry` names and run it; its stdout.
fn run(dir: &Path, entry: &str, name: &str) -> String {
    let bin = harness::unique_bin(name);
    let built = build_opts::build_seed_dir(&dir.join(entry), &bin, &build_opts::options());
    if let Err(e) = built {
        let _ = std::fs::remove_dir_all(dir);
        panic!("build {entry}: {e:?}");
    }
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "{entry} exits 0: {:?}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const STORE: &str = "\
type ModelRow { id: Int; name: String; }
type Side = enum { Buy, Sell };
type Box<T> { v: T; }
const LIMIT: Int = 7;
fn describe(r: ModelRow) -> String { return r.name + \"#\" + to_string(r.id); }
fn flip(s: Side) -> Side { return match s { Side::Buy -> Side::Sell, Side::Sell -> Side::Buy, }; }
";

/// Every listable kind used bare: a record in a literal, an annotation
/// and a signature, an enum's variants in construction and in a match,
/// a generic record at `Box<Int>`, a const, a fn; and `as` renaming one,
/// with the qualified spelling beside it.
#[test]
fn listed_names_are_bare_in_every_file_of_the_seed() {
    let dir = seeds(
        "import_lists_bare",
        &[
            ("store/store.hl", STORE),
            (
                "app/main.hl",
                "import \"../store\" as st { ModelRow, Side, Box, LIMIT, flip, describe as show };\n\
                 fn first(rows: [ModelRow; 2]) -> ModelRow { return rows[0]; }\n\
                 fn main() {\n\
                     let r: ModelRow = first([ModelRow { id: 1, name: \"a\" }, st::ModelRow { id: 2, name: \"b\" }]);\n\
                     println(show(r));\n\
                     let b: Box<Int> = Box { v: LIMIT };\n\
                     println(to_string(b.v + st::LIMIT));\n\
                     println(match flip(Side::Buy) { Side::Sell -> \"sell\", Side::Buy -> \"buy\", });\n\
                     println(from_sibling());\n\
                 }\n",
            ),
            // The list is the seed's: a sibling file spells the names bare
            // without an import of its own.
            ("app/other.hl", "fn from_sibling() -> String { let ModelRow = 5; return describe_row(ModelRow); }\nfn describe_row(n: Int) -> String { return show(st::ModelRow { id: n, name: \"sib\" }); }\n"),
        ],
    );
    let out = run(&dir, "app", "import_lists_bare");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, ["a#1", "14", "sell", "sib#5"], "stdout: {out:?}");
}

/// A list re-exports: `api` lists `ModelRow` and `Side` from `store`,
/// and its importer reaches them as `api::ModelRow` (and lists one of
/// them again), which is `st::ModelRow` — a value made under one
/// spelling is passed to a fn that takes the other, compared, and
/// returned, with no conversion. The rename table gives both spellings
/// one symbol.
#[test]
fn a_listed_name_is_reexported_as_the_same_declaration() {
    let dir = seeds(
        "import_lists_reexport",
        &[
            ("store/store.hl", STORE),
            (
                "api/api.hl",
                "import \"../store\" as st { ModelRow, Side, describe as show };\n\
                 fn make(id: Int) -> ModelRow { return ModelRow { id: id, name: \"row\" }; }\n\
                 fn label(r: ModelRow) -> String { return show(r); }\n",
            ),
            (
                "app/main.hl",
                "import \"../api\" as api { ModelRow };\n\
                 import \"../store\" as st;\n\
                 fn back(r: st::ModelRow) -> api::ModelRow { return r; }\n\
                 fn main() {\n\
                     let r: api::ModelRow = api::make(3);\n\
                     println(st::describe(r));\n\
                     let mine: ModelRow = st::ModelRow { id: 4, name: \"mine\" };\n\
                     println(api::label(back(mine)));\n\
                     let s: api::Side = st::Side::Buy;\n\
                     println(to_string(s == api::Side::Buy));\n\
                 }\n",
            ),
        ],
    );
    let snap = Snapshot::load(
        &dir.join("app"),
        LoadMode::WholeSeed,
        &Disk,
        build_opts::harness_config(&build_opts::options()),
    )
    .unwrap_or_else(|_| panic!("load the seed"));
    let row = |alias: &str, name: &str| {
        snap.import_renames()
            .iter()
            .find(|(k, _)| k[..] == [alias.to_string(), name.to_string()])
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| panic!("a row for {alias}::{name}: {:?}", snap.import_renames()))
    };
    assert_eq!(row("api", "ModelRow"), row("st", "ModelRow"), "one symbol under both aliases");
    assert_eq!(row("api", "Side"), row("st", "Side"));
    assert!(
        !snap.import_renames().iter().any(|(k, _)| k[..] == ["api".to_string(), "describe".to_string()]),
        "a renamed item re-exports under its new name only"
    );
    assert_eq!(row("api", "show"), row("st", "describe"));
    drop(snap);
    let out = run(&dir, "app", "import_lists_reexport");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, ["row#3", "mine#4", "true"], "stdout: {out:?}");
}

/// A topic listed from another seed is subscribed and published bare,
/// with its payload type listed beside it.
#[test]
fn a_listed_topic_is_subscribed_and_published_bare() {
    let dir = seeds(
        "import_lists_topic",
        &[
            ("source/source.hl", "type Beat { n: Int; }\ntopic Heartbeat { payload: Beat; subject: \"t.beat\"; }\n"),
            (
                "app/main.hl",
                "import \"../source\" as source { Heartbeat, Beat };\n\
                 locus Sub {\n\
                     params { count: Int = 0; }\n\
                     bus { subscribe Heartbeat as on_beat; }\n\
                     fn on_beat(b: Beat) {\n\
                         self.count = self.count + 1;\n\
                         println(\"got n=\" + b.n);\n\
                     }\n\
                 }\n\
                 locus Pub {\n\
                     bus { publish Heartbeat; }\n\
                     run() {\n\
                         Heartbeat <- Beat { n: 42 };\n\
                         Heartbeat <- source::Beat { n: 99 };\n\
                     }\n\
                 }\n\
                 fn main() {\n\
                     let s = Sub { };\n\
                     Pub { };\n\
                     println(\"done\");\n\
                 }\n",
            ),
        ],
    );
    let out = run(&dir, "app", "import_lists_topic");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.contains("got n=42"), "the first beat: {out:?}");
    assert!(out.contains("got n=99"), "the second beat: {out:?}");
    assert!(out.contains("done"), "{out:?}");
}
