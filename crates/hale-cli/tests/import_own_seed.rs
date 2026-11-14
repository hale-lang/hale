//! A seed does not import itself (spec/projects.md § Seeds).
//!
//! In a directory seed holding `main.hl` and `util.hl`,
//! `import "util" as util;` resolved (resolution rule 1 finds
//! `util.hl`) and said nothing, but `util.hl` is a file of the seed
//! being loaded: it was already parsed, so the import parsed nothing,
//! bound no rename row under the alias, and every `util::double(2)` was
//! refused as "`util` is not an import or a type of this seed" — an
//! error at each use naming the alias as unknown, with the import that
//! caused it silent (downstream handoff). `import "./util"` and
//! `import "."` behaved the same.
//!
//! The import is refused now, at its path literal, saying the file's
//! declarations are already in scope by their bare names. The same
//! sibling imported from a single-FILE target (`hale run main.hl`,
//! whose seed is that file alone) is a single-file library under rule
//! 1, and still is.

use std::path::{Path, PathBuf};
use std::process::Command;

fn seed(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_import_own_seed_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, src).expect("write");
    }
    d
}

fn hale(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

const UTIL: &str = "fn double(x: Int) -> Int { return x * 2; }\n";

fn main_importing(path: &str) -> String {
    format!("import \"{path}\" as util;\nfn main() {{ println(to_string(util::double(2))); }}\n")
}

/// The refusal, on every compiling verb, located at the import's path
/// literal; no use of the alias is reported, since the import is the
/// line that has to change.
#[test]
fn a_file_of_the_importing_seed_is_refused_at_the_import() {
    for (tag, path) in [("bare", "util"), ("dotted", "./util")] {
        let d = seed(tag, &[("main.hl", &main_importing(path)), ("util.hl", UTIL)]);
        for verb in ["check", "build", "run"] {
            let (ok, out) = hale(&d, &[verb, "."]);
            assert!(!ok, "`hale {verb}` refuses the import of a file of its own seed:\n{out}");
            let want = format!(
                "main.hl:1:8: type error: `{path}` names util.hl, a file of this seed: a seed's files share one scope, \
                 so its declarations are already in scope by their bare names, and a seed does not import itself — \
                 call them as `name(...)`, not `alias::name(...)`, and drop this import"
            );
            assert!(out.contains(&want), "`hale {verb}` ({path}): the located refusal\n  want: {want}\n  got:\n{out}");
            assert!(
                !out.contains("is not an import or a type of this seed"),
                "`hale {verb}` ({path}): the alias's uses are not what is reported:\n{out}"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// `import "."` names the seed's own directory: refused the same way.
#[test]
fn the_importing_seeds_own_directory_is_refused_at_the_import() {
    let d = seed("dir", &[("main.hl", &main_importing(".")), ("util.hl", UTIL)]);
    let (ok, out) = hale(&d, &["check", "."]);
    assert!(!ok, "check refuses it:\n{out}");
    assert!(
        out.contains(
            "main.hl:1:8: type error: `.` names this seed's own directory: its declarations are already in scope by \
             their bare names, and a seed does not import itself"
        ),
        "the located refusal:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The neighbours that stay as they were: the bare names in the
/// directory seed, and the sibling imported from a single-file target,
/// whose seed is the file alone.
#[test]
fn bare_names_in_the_seed_and_a_sibling_of_a_file_target_still_run() {
    let d = seed(
        "neighbours",
        &[("main.hl", "fn main() { println(to_string(double(2))); }\n"), ("util.hl", UTIL)],
    );
    let (ok, out) = hale(&d, &["run", "."]);
    assert!(ok && out.lines().any(|l| l == "4"), "the seed's own names, bare:\n{out}");
    let _ = std::fs::remove_dir_all(&d);

    let d = seed("file_target", &[("main.hl", &main_importing("util")), ("util.hl", UTIL)]);
    let (ok, out) = hale(&d, &["run", "main.hl"]);
    assert!(ok && out.lines().any(|l| l == "4"), "a single-file target's sibling is a library of its own:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}
