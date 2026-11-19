//! F.40 phase 3, E0: the entry decision, as `hale check` says it.
//!
//! The entry row (`hale_types::entry`) is the one answer to which
//! `main locus` is the program's entry, and the checker reads it for
//! rule 1's count and rule 9's closed world; the pool map (F.31) and
//! the pinned-in-a-loop rule read the entry the placement table is
//! seeded from. Before the row, the four disagreed, and the test here
//! pins what the row decided: an imported `main` is not the entry. A
//! seed whose only `main locus` came in through an `import` checks as a
//! seed with no `main` does: its library's placement seeds no pool map
//! here, so a cross-pool call inside the library is the library's own
//! finding, not the importer's (before the row, the pool map took the
//! imported `main` and reported it through the importer).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A `main locus` whose `run()` calls a method on a pinned field: the
/// cross-pool error (F.31) is the evidence that the pool map took it.
const PLACED: &str = "\
type Msg { v: Int = 0; }

topic Out { payload: Msg; subject: \"entry.out\"; }

locus Worker {
    fn work() { }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: pinned; }
    bus { publish Out; }
    run() {
        self.w.work();
        Out <- Msg { v: 1 };
    }
}
";

const CROSS_POOL: &str = "cross-pool method call";
const NO_MAIN: &str = "fn main() { }\n";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_check_entry_{}_{}", std::process::id(), tag));
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

fn check(seed: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(seed)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

/// What `hale check` says of a seed with no `main locus` at all.
const NO_MAIN_REPORT: &str = "ok: 1 file(s) typechecked\n";

#[test]
fn a_seed_with_no_main_checks_clean() {
    let root = scratch("none");
    let (ok, out) = check(&seed(&root, "app", NO_MAIN));
    assert!(ok, "{out}");
    assert_eq!(out, NO_MAIN_REPORT);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn decision_1_an_imported_main_is_not_the_entry() {
    let root = scratch("imported");
    let lib = seed(&root, "lib", &format!("{PLACED}\nfn main() {{ App {{ }}; }}\n"));
    // The control: the library checked as its own entry reports the
    // cross-pool call.
    let (ok, out) = check(&lib);
    assert!(!ok && out.contains(CROSS_POOL), "the library's own check: {out}");
    // A seed whose only `main locus` is the library's has no entry, and
    // says what a seed with no `main` says.
    let app = seed(&root, "app", &format!("import \"../lib\" as lib;\n{NO_MAIN}"));
    let (ok, out) = check(&app);
    assert!(ok, "the importer has no entry, so nothing of the library is placed here: {out}");
    assert_eq!(out, NO_MAIN_REPORT);
    let _ = std::fs::remove_dir_all(&root);
}

