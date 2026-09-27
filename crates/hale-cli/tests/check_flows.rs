//! GH #736 — `hale check --flows` names the `release(c: T)` clause(s) that
//! make each locus type a flow. Whether `T` is a flow is decided over the
//! whole program, imported seeds included, and a clause on a locus that is
//! never instantiated counts all the same; the report names each clause in
//! the spelling the author wrote (`lib::Waiter`, not its mangled symbol)
//! with its file and line, and says when there is none. It is a report,
//! never a warning: a program that declares a release still checks clean.

use std::path::{Path, PathBuf};
use std::process::Command;

// `files` is `(relative path, source)` pairs; `main.hl` is the target.
fn check(files: &[(&str, &str)], flags: &[&str], tag: &str) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_flows_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (rel, src) in files {
        let f = d.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, src).unwrap();
    }
    let target = d.join("main.hl").to_string_lossy().to_string();
    let mut args: Vec<&str> = vec!["check"];
    args.extend_from_slice(flags);
    args.push(&target);
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(&args).current_dir(Path::new("/")).output().expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

// An imported seed declares the release on a locus nothing instantiates.
const LIB: &str = "locus Waiter {\n    run() { }\n}\n\nlocus Keeper {\n    accept(w: Waiter) { }\n    release(w: Waiter) { }\n}\n";

const MAIN_IMPORTS: &str = "import \"./lib\" as lib;\n\nfn main() { println(\"checked\"); }\n";

// The program's own release of its own child, beside the imported one.
const MAIN_OWN: &str = "import \"./lib\" as lib;\n\nlocus Job {\n    run() { }\n}\n\nlocus Runner {\n    accept(j: Job) { }\n    release(j: Job) { }\n}\n\nfn main() { println(\"checked\"); }\n";

const NONE: &str = "locus Resident {\n    run() { }\n}\n\nlocus Holder {\n    accept(r: Resident) { }\n}\n\nfn main() { println(\"checked\"); }\n";

#[test]
fn a_release_in_an_imported_seed_is_named_with_its_file_and_line() {
    let (ok, out) = check(&[("main.hl", MAIN_IMPORTS), ("lib/waiter.hl", LIB)], &["--flows"], "imported");
    assert!(ok, "a report, never a failure: {out}");
    assert!(out.contains("flows: 1 locus type(s) reclaimed when their run() completes"), "{out}");
    assert!(out.contains("lib::Waiter — a flow, by:"), "the flow type as the author spells it: {out}");
    assert!(out.contains("release(w: lib::Waiter) in lib::Keeper") && out.contains("lib/waiter.hl:7:"), "the clause, its owner and where it is: {out}");
    assert!(out.contains("whether or not its declaring locus is instantiated"), "the type-wide rule, said: {out}");
    assert!(!out.contains("__lib"), "no mangled symbol: {out}");
}

#[test]
fn every_clause_is_listed_by_type_and_the_program_s_own_beside_the_imported() {
    let (ok, out) = check(&[("main.hl", MAIN_OWN), ("lib/waiter.hl", LIB)], &["--flows"], "both");
    assert!(ok, "{out}");
    assert!(out.contains("flows: 2 locus type(s)"), "{out}");
    assert!(out.contains("Job — a flow, by:") && out.contains("release(j: Job) in Runner") && out.contains("main.hl:9:"), "the program's own: {out}");
    assert!(out.contains("release(w: lib::Waiter) in lib::Keeper"), "and the imported: {out}");
}

#[test]
fn without_a_release_every_child_is_a_resident_and_no_flag_no_report() {
    let (ok, out) = check(&[("main.hl", NONE)], &["--flows"], "none");
    assert!(ok, "{out}");
    assert!(out.contains("flows: none — every accept'd child is a resident, reclaimed when its owner dissolves"), "{out}");
    // the report is asked for: a release declaration alone says nothing
    let (ok, out) = check(&[("main.hl", MAIN_IMPORTS), ("lib/waiter.hl", LIB)], &[], "quiet");
    assert!(ok && !out.contains("flows:") && !out.contains("warning"), "no blanket notice on a legitimate release: {out}");
}
