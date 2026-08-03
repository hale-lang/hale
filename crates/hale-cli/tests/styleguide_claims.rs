//! `spec/styleguide.md` §7 claims that are about CODEGEN limits.
//!
//! Companion to `hale-types/tests/styleguide_snippets.rs`, which pins
//! the claims a typechecker can see. These two cannot live there,
//! because `hale check` does not see them at all — they surface only
//! at `hale build`.
//!
//! That is worth stating plainly, because it is the same shape as the
//! unknown-`std::`-namespace hole (#353 item 9): a reader who trusts
//! the checker is told nothing is wrong. §7's two headline absences
//! both behave this way, so a styleguide reader who writes
//! `Vec<User>` and runs `hale check` gets a clean bill of health and
//! then a build error.
//!
//! Each test asserts the CURRENT truth. If either gap closes, the
//! test fails and points at the styleguide entry to update.

use std::process::Command;

fn build_fails(src: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir()
        .join(format!("hale-sgclaim-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&f)
        .output()
        .expect("run hale build");
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (!out.status.success(), text)
}

/// §7: "the *mechanism* IS missing … a generic payload enum declares
/// and compiles, but does not construct."
///
/// This entry was WRONG for months in the other direction — it said
/// generic enums "compile, construct, and match today". Nothing
/// checked it, so anyone following the styleguide hit an unsupported
/// error. Pinned now in both directions.
#[test]
fn claim_generic_payload_enums_do_not_construct() {
    // Non-generic constructs. Genericity is the whole difference, and
    // asserting that here is what makes a future failure legible.
    let (ng_failed, ng_out) = build_fails(
        "type Res = enum { Ok(Int), Err(String) };\n\
         fn main() { let r = Res::Ok(1);\n\
             match r { Res::Ok(n) -> println(n), Res::Err(m) -> println(m) } }",
        "nongeneric",
    );
    assert!(
        !ng_failed,
        "a NON-generic payload enum must still construct; if this \
         breaks, §7's framing is wrong in a new way:\n{}",
        ng_out
    );

    let (failed, out) = build_fails(
        "type Opt<T> = enum { Some(T), None };\n\
         fn main() { let r = Opt::Some(1);\n\
             match r { Opt::Some(n) -> println(n), Opt::None -> println(0) } }",
        "generic",
    );
    assert!(
        failed,
        "spec/styleguide.md §7 says a generic payload enum does not \
         construct. It now does — update that entry, and consider \
         whether `Option<T>` should ship:\n{}",
        out
    );
}

/// §7: "No parametric collection types (`List<T>` / `Map<K,V>`).
/// Collections are loci."
#[test]
fn claim_no_parametric_collection_types() {
    let (failed, out) = build_fails(
        "type User { active: Bool; }\n\
         fn f(v: Vec<User>) -> Int { return 1; }\n\
         fn main() { println(1); }",
        "vec",
    );
    assert!(
        failed,
        "spec/styleguide.md §7 says there are no parametric collection \
         types. `Vec<User>` now resolves — update that entry:\n{}",
        out
    );
}

/// The shape that makes both of the above worth pinning HERE rather
/// than in the typecheck harness: `hale check` accepts them.
#[test]
fn these_gaps_are_invisible_to_check() {
    let dir = std::env::temp_dir()
        .join(format!("hale-sgvis-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(
        &f,
        "type Opt<T> = enum { Some(T), None };\n\
         fn main() { let r = Opt::Some(1);\n\
             match r { Opt::Some(n) -> println(n), Opt::None -> println(0) } }",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&f)
        .output()
        .expect("run hale check");
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("typechecked"),
        "documenting the current split: `hale check` accepts a generic \
         enum construction that `hale build` rejects. If check learns \
         to reject it, delete this test and move the claim into the \
         typecheck harness — the split is the thing being recorded, \
         not a behaviour worth preserving:\n{}",
        text
    );
}
