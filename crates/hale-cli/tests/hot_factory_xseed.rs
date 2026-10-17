//! GH #402: the hot-path advisory across a seed boundary.
//!
//! This lives at the CLI layer because the behavior only exists once
//! seeds are merged. At a cross-seed call the callee keeps its author
//! spelling (`mat::zeros`) while the symbol is merged under a mangled
//! one (`__lib_<id>_<stem>_<fn>`). The lint takes the callee the
//! allocation summary resolved the call to, by declaration, so a
//! single-source unit test, where resolution is exact either way,
//! cannot pin any of this.
//!
//! Three properties:
//!
//!  - a cross-seed factory call is flagged, in author spelling. Without
//!    this the lint would be silent on exactly the codebases it was
//!    written for (#402's reference workload calls its factories
//!    cross-seed).
//!  - two seeds exporting the same name, one returning a locus and one
//!    not, are told apart by the alias the call is written through:
//!    `mat::build` is flagged, `plain::build` is not. The lint used to
//!    match a path's last segment against the tail of every mangled
//!    name, and dropped such a name as ambiguous.
//!  - a stdlib call is never taken for a program's fn. The tail match
//!    split a mangled name at `_`, so an imported `one_arg` returning a
//!    locus made every `let a = std::env::arg(i)` in a loop a hot-path
//!    allocation of that locus (downstream handoff).

use std::path::PathBuf;
use std::process::Command;

fn check_app() -> String {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hot-factory-app");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&app)
        .output()
        .expect("run hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_cross_seed_factory_call_in_a_loop_is_flagged() {
    let out = check_app();
    assert!(
        out.contains("`mat::zeros` returns the locus"),
        "a factory call must resolve across the seed boundary, in \
         author spelling on both halves:\n{}",
        out
    );
    assert!(
        !out.contains("__lib_"),
        "no mangled symbol may reach the author:\n{}",
        out
    );
}

#[test]
fn the_alias_names_the_declaration_a_shared_name_reaches() {
    let out = check_app();
    assert!(
        out.contains("`mat::build` returns the locus"),
        "`mat::build` is the library's factory, whatever another seed \
         names `build`:\n{}",
        out
    );
    assert!(
        !out.contains("`plain::build`"),
        "`plain::build` returns an Int: no finding:\n{}",
        out
    );
}

#[test]
fn a_stdlib_call_is_never_a_program_factory() {
    let out = check_app();
    assert!(
        !out.contains("std::env::arg"),
        "`std::env::arg` is not the library's `one_arg`:\n{}",
        out
    );
    assert!(
        out.contains("returns the locus"),
        "the factory calls in the same file must still fire:\n{}",
        out
    );
}
