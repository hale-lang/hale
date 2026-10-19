//! The built-in group `program` and the inline gate `std::log::*`,
//! adopted from a policy seed's constitution.
//!
//! A standard-practices pack states its law in a seed that closes no
//! world, and every entrypoint that adopts it is judged in its own
//! world. `program` is what lets that law name "everything this
//! entrypoint declares" without asking each adopter to declare a group:
//! it resolves where the clause is evaluated, to the adopting
//! entrypoint's own seed and nothing it imports.

use std::path::PathBuf;
use std::process::Command;

fn check(seed: &str) -> String {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claims-program")
        .join(seed);
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&fixture)
        .output()
        .expect("run hale check");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// The policy seed checks on its own, and an adopter that logs through
/// `std::log` holds the law, although a seed it imports prints: the
/// imported seed's declarations are not the entrypoint's own.
#[test]
fn an_adopter_that_logs_through_std_log_holds_the_policy_law() {
    for seed in ["policy", "app-pass"] {
        let out = check(seed);
        assert!(!out.contains("type error"), "`{seed}`:\n{out}");
    }
}

/// An adopter that calls the printing seed violates the law, and the
/// witness names the path out of the program through the import.
#[test]
fn an_adopter_that_calls_a_printing_seed_violates_with_the_path() {
    let out = check("app-fail");
    assert!(out.contains("claim `logs_structured` violated"), "{out}");
    assert!(
        out.contains("`program` reaches `effects(stdio)` — witness: `App::run` -> `shout::shout`"),
        "{out}"
    );
}
