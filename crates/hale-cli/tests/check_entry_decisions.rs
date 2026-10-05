//! F.40 phase 3, E0: the two entry decisions, as `hale check` says them.
//!
//! The entry row (`hale_types::entry`) is the one answer to which
//! `main locus` is the program's entry, and the checker reads it for
//! rule 1's count and rule 9's closed world; the pool map (F.31) and
//! the pinned-in-a-loop rule read the root the placement table is
//! seeded from. Before the row, the four disagreed on two shapes, and
//! each test here pins what the row decided:
//!
//! 1. An imported `main` is not the entry. A seed whose only `main
//!    locus` came in through an `import` checks as a seed with no
//!    `main` does: its library's placement seeds no pool map here, so
//!    a cross-pool call inside the library is the library's own
//!    finding, not the importer's (before the row, the pool map took
//!    the imported `main` and reported it through the importer).
//! 2. A module-nested `main` is not the entry: a seed with both keeps
//!    the top-level one, and rule 1 still counts both. A seed whose
//!    only `main locus` is inside a `module { }` has no entry, so rule
//!    9's world is not closed, and lowering, which deploys the entry,
//!    has nothing to deploy: the check refuses it at that locus's name
//!    (F.40 phase 3, L4). The refusal is one more diagnostic: every
//!    other rule still judges the nested `main`, so its cross-pool call
//!    is refused as the top-level one is (GH #825; the outside review
//!    of #1293, finding 1). The end-to-end half is
//!    `nested_main_transition.rs`.

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
const REFUSED: &str = "the entry must be top-level: `main locus App` inside `module inner` is not the program's \
                       entry, and nothing else in the seed is — move it out of the module";
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

#[test]
fn decision_2_a_module_nested_main_is_not_the_entry_and_is_still_placed() {
    let root = scratch("nested");
    // The control: the same declarations at the top level.
    let (ok, flat) = check(&seed(&root, "flat", &format!("{PLACED}\nfn main() {{ App {{ }}; }}\n")));
    assert!(!ok && flat.contains(CROSS_POOL), "the top-level control: {flat}");
    assert!(!flat.contains(REFUSED), "the top-level control is the entry: {flat}");
    let body: String = PLACED.lines().map(|l| format!("    {l}\n")).collect();
    let nested = seed(&root, "nested", &format!("module inner {{\n{body}}}\n\nfn main() {{ App {{ }}; }}\n"));
    let (ok, out) = check(&nested);
    // Not the entry, and nothing else is: refused at its name, line 10
    // (`    main locus App {`, the name at column 16).
    assert!(!ok, "{out}");
    assert_eq!(out.matches(REFUSED).count(), 1, "the refusal, once: {out}");
    assert!(out.contains(&format!("main.hl:10:16: type error: {REFUSED}")), "located at the name: {out}");
    // One more diagnostic, never a replacement: its pinned field is
    // still placed, so the direct call is cross-pool.
    assert!(out.contains(CROSS_POOL), "a module-nested main locus is still judged: {out}");
    // Not the entry: the world is not closed, so the orphan `Out` the
    // top-level control reports is not reported here.
    let orphan = "bus topic `Out` is published but has no subscriber";
    assert!(flat.contains(orphan), "{flat}");
    assert!(!out.contains(orphan), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn decision_2_a_seed_with_both_keeps_the_top_level_main() {
    let root = scratch("both");
    let body: String = PLACED.replace("main locus App", "main locus Other").lines().map(|l| format!("    {l}\n")).collect();
    let both = seed(
        &root,
        "both",
        &format!("main locus App {{ params {{ n: Int = 0; }} }}\n\nmodule inner {{\n{body}}}\n\nfn main() {{ App {{ }}; }}\n"),
    );
    let (ok, out) = check(&both);
    assert!(!ok, "{out}");
    // Rule 1 counts the module-nested one (GH #825).
    assert!(out.contains("more than one `main` locus declared (`App` is one of 2)"), "{out}");
    assert!(out.contains("more than one `main` locus declared (`Other` is one of 2)"), "{out}");
    // The entry is the top-level `App`, and so is lowering's root (the
    // first `main` in declaration order), whose placement is empty: the
    // nested `Other`'s pinned field is placed by nothing, so its call is
    // not cross-pool (before the row, the pool map took the last `main`
    // in declaration order, `Other`, which lowering does not deploy).
    assert!(!out.contains(CROSS_POOL), "{out}");
    // The closed world is the entry's: `Out` is still an orphan.
    assert!(out.contains("bus topic `Out` is published but has no subscriber"), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}
