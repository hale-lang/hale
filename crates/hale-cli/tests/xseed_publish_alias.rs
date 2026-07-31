//! A `publish:` contract written in a LIBRARY must survive being
//! imported (#330 groundwork).
//!
//! Subjects reach the analysis as the import resolver's MANGLED
//! symbol — `__lib_lib_relay_main_Recalled` — while the annotation
//! holds the source text `Recalled`. The comparison was exact string
//! equality, so a library's own publish contract became unsatisfiable
//! the moment anyone imported it.
//!
//! The failure pointed the worst possible direction: the library
//! passed `hale check` standalone and failed only in the consumer's
//! build, naming a symbol the library author never wrote and cannot
//! predict — the mangled name embeds the IMPORTER's chosen alias.
//!
//! Found while building the cross-seed launderer fixture for RFC #330.
//! `depends:` names topics the same way, so it would have inherited
//! this verbatim.

use std::path::PathBuf;
use std::process::Command;

fn check(dir: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir))
        .output()
        .expect("invoke hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn library_publish_contract_holds_when_imported() {
    let standalone = check("tests/fixtures/xseed-launderer/lib/relay");
    assert!(
        standalone.contains("typechecked"),
        "library must check standalone: {}",
        standalone
    );
    let imported = check("tests/fixtures/xseed-launderer/app");
    assert!(
        !imported.contains("declared publish set violated"),
        "a library's own publish contract must survive import — the \
         subject arrives mangled with the importer's alias: {}",
        imported
    );
    assert!(
        imported.contains("typechecked"),
        "the cross-seed launderer fixture should check clean: {}",
        imported
    );
}
