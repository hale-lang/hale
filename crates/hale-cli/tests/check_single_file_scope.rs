//! GH #1139: `hale check <file>` and `hale check <seed>` report the
//! same diagnostic for a read of a block's `let` after the block.
//!
//! One file of a multi-file seed checked alone keeps the permissive
//! reading for a bare identifier nothing binds, since it may be a
//! `const` a sibling declares (GH #721). A name a block of the same
//! body bound and released is not that: the block's binding was the
//! block's (GH #1132), so the single-file check refuses it as the seed
//! check, `hale build` and `hale run` do.

use std::path::{Path, PathBuf};
use std::process::Command;

const SRC: &str = "fn f(take: Bool) -> Int {\n    if take {\n        let x = 1;\n    }\n    return x;\n}\nfn main() { println(f(true)); }\n";
const MSG: &str = "unknown identifier `x`";
const ENDED: &str = "bound in a block that has ended";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_single_file_scope_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

#[test]
fn the_single_file_check_refuses_it_as_the_seed_check_does() {
    let d = scratch("both");
    let file = d.join("scope1.hl");
    std::fs::write(&file, SRC).unwrap();
    let seed = d.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), SRC).unwrap();

    let (ok, out) = hale(&["check", &file.to_string_lossy()]);
    assert!(!ok, "the single-file check must fail: {out}");
    assert!(out.contains(MSG) && out.contains(ENDED) && out.contains(":5:12:"), "at the read, naming the ended block: {out}");

    let (ok, out) = hale(&["check", &seed.to_string_lossy()]);
    assert!(!ok, "the seed check must fail: {out}");
    assert!(out.contains(MSG) && out.contains(":5:12:"), "{out}");

    let (ok, out) = hale(&["build", &file.to_string_lossy()]);
    assert!(!ok, "the build must refuse it too: {out}");
    assert!(out.contains(MSG), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The leniency itself stays: one file reading a name it never bound
/// still checks clean alone, since a sibling may declare it.
#[test]
fn a_name_the_file_never_bound_stays_lenient_alone() {
    let d = scratch("lenient");
    let file = d.join("part.hl");
    std::fs::write(&file, "fn g() -> Int { return LIMIT; }\n").unwrap();
    let (ok, out) = hale(&["check", &file.to_string_lossy()]);
    assert!(ok, "a sibling's const is not the file's to refuse: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A name closed two blocks up is still the block's, not a sibling's
/// const: `closed` is not cleared until the body's own frame pops, so
/// nesting the `let` one level deeper still trips it.
#[test]
fn a_name_closed_two_levels_up_is_still_refused() {
    const NESTED_SRC: &str = "fn f(a: Bool, b: Bool) -> Int {\n    if a {\n        if b {\n            let x = 1;\n        }\n    }\n    return x;\n}\nfn main() { println(f(true, true)); }\n";
    let d = scratch("nested");
    let file = d.join("scope2.hl");
    std::fs::write(&file, NESTED_SRC).unwrap();
    let (ok, out) = hale(&["check", &file.to_string_lossy()]);
    assert!(!ok, "the single-file check must fail: {out}");
    assert!(out.contains(MSG) && out.contains(ENDED) && out.contains(":7:12:"), "at the read, naming the ended block: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A block that shadows an outer `let` does not poison it: the outer
/// binding is still in scope after the inner block ends, so the read
/// resolves to it — `closed` is only consulted once `lookup` fails.
#[test]
fn shadowing_in_a_nested_block_does_not_poison_the_outer_binding() {
    const SHADOW_SRC: &str = "fn g(take: Bool) -> Int {\n    let x = 1;\n    if take {\n        let x = 2;\n    }\n    return x;\n}\nfn main() { println(g(true)); }\n";
    let d = scratch("shadow");
    let file = d.join("scope3.hl");
    std::fs::write(&file, SHADOW_SRC).unwrap();
    let (ok, out) = hale(&["check", &file.to_string_lossy()]);
    assert!(ok, "the outer `x` is still in scope after the inner block ends: {out}");
    let _ = std::fs::remove_dir_all(&d);
}
