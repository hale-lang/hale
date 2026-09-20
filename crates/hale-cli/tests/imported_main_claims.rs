//! GH #733 — an imported main's inline claims keep their own seed's
//! groups.
//!
//! A `main locus` may state its law inline, and the groups it names
//! are declared beside it, in the same seed. Another seed importing
//! that application merges it into the closing world, where the
//! imported main is still the only main — so its inline claims are
//! the world law and are re-evaluated here. The group NAMES had to
//! travel with them: group declarations are mangled at the import
//! (GH #382), so an unrewritten reference detached from the
//! declaration it was written against and the importer reported the
//! defining seed's own groups as "never declared".
//!
//! The obligations this pins:
//!   - direct and imported checks of the SAME inline claim agree;
//!   - a real violating path is rejected both ways (the fix is not
//!     "resolve to nothing and pass");
//!   - a genuinely undeclared group still errors, both ways;
//!   - an importer's same-named group is not substituted for the
//!     one the claim was written against;
//!   - the adopted-constitution form, which already worked through
//!     an import, still does.
//!
//! GH #774 extends the third of those to the case the rename table
//! could not reach. A reference NO declaration in the defining seed
//! answers has nothing to be rewritten to, so it used to travel as
//! written — and an importer that happened to declare a group of
//! that name captured it, which made the defining seed's
//! unknown-group error disappear and evaluated the law against a
//! stranger's group. It is now bound to the seed that wrote it, in
//! all three tiers a claim can travel in: an inline main-locus
//! claim, a library-tier `claims { }` block, and a `constitution`
//! declared by a seed that closes a world. The last of those has a
//! carve-out with a test of its own — a POLICY seed's constitution
//! is adopted elsewhere, and its vocabulary belongs to whoever
//! adopts it.

use std::path::PathBuf;
use std::process::Command;

fn check(seed: &str) -> String {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/imported-main-claims")
        .join(seed);
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&fixture)
        .output()
        .expect("run hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The bug: the importer rejected `participants` and `rooms`, which
/// its defining seed declares right beside the claim.
#[test]
fn direct_and_imported_checks_of_an_inline_claim_agree() {
    let direct = check("seed");
    assert!(
        !direct.contains("type error"),
        "the defining seed must check clean:\n{}",
        direct
    );
    let imported = check("app");
    assert!(
        !imported.contains("never declared"),
        "the imported main's inline claim must resolve its own \
         seed's groups:\n{}",
        imported
    );
    assert!(
        !imported.contains("type error"),
        "importing a valid application must stay valid:\n{}",
        imported
    );
}

/// Resolving in the defining seed may not mean resolving to nothing:
/// the law still has to fire on a real path.
#[test]
fn a_violating_path_is_rejected_directly_and_through_an_import() {
    for seed in ["seed-bad", "app-bad"] {
        let out = check(seed);
        assert!(
            out.contains("claim `guests_sign_only_via_rooms` violated"),
            "`{}`: the self-signing participant must violate the \
             claim:\n{}",
            seed,
            out
        );
        assert!(
            out.contains("Participant::run")
                && out.contains("SessionSigner::stamp"),
            "`{}`: the witness must name the crossing path:\n{}",
            seed,
            out
        );
    }
    assert!(
        !check("app-bad").contains("__lib_"),
        "no mangled symbol may leak into the imported diagnostic"
    );
}

/// An unknown group is still an unknown group — through an import it
/// must not become vacuously true.
#[test]
fn a_genuinely_missing_group_still_errors_both_ways() {
    for seed in ["seed-missing", "app-missing"] {
        let out = check(seed);
        assert!(
            out.contains(
                "claim `guests_sign_nothing` names group `guests`, \
                 which is never declared"
            ),
            "`{}`: the unknown-group diagnostic must stand:\n{}",
            seed,
            out
        );
    }
}

/// GH #774 — the capture, in every tier a claim travels in.
///
/// `direct` is the defining seed on its own; `importer` declares a
/// group of the missing name over an innocent locus of its own. The
/// error is the SAME error in both builds, and it names the group the
/// author wrote — a seed-bound reference is unspeakable in source, so
/// letting its synthesized spelling reach the message would point at
/// something that appears nowhere.
fn an_undeclared_group_stays_undeclared(
    direct: &str,
    importer: &str,
    claim: &str,
) {
    for seed in [direct, importer] {
        let out = check(seed);
        assert!(
            out.contains(&format!(
                "claim `{}` names group `guests`, which is never \
                 declared",
                claim
            )),
            "`{}`: an undeclared group is an error in the defining \
             seed regardless of what an importer declares:\n{}",
            seed,
            out
        );
        assert!(
            !out.contains("__lib_") && !out.contains("__unbound"),
            "`{}`: no synthesized spelling may reach the \
             diagnostic:\n{}",
            seed,
            out
        );
    }
}

/// An inline main-locus claim (the GH #733 surface).
#[test]
fn an_inline_claims_undeclared_group_is_not_captured() {
    an_undeclared_group_stays_undeclared(
        "seed-missing",
        "app-missing-collision",
        "guests_sign_nothing",
    );
}

/// A library-tier `claims { }` block, which travels by design.
#[test]
fn a_library_tier_undeclared_group_is_not_captured() {
    an_undeclared_group_stays_undeclared(
        "seed-lib-missing",
        "app-lib-missing",
        "guests_never_stamp",
    );
}

/// An adopted `constitution` — the form GH #733's workaround moved
/// law INTO, so it is the one most likely to be travelling.
#[test]
fn a_constitutions_undeclared_group_is_not_captured() {
    an_undeclared_group_stays_undeclared(
        "seed-const-missing",
        "app-const-missing",
        "guests_sign_nothing",
    );
}

/// The carve-out, and the reason the constitution rule above is
/// about the seed and not about the keyword: a POLICY seed closes no
/// world, so its constitution is adopted elsewhere and its group
/// vocabulary is the ADOPTING entrypoint's to declare (spec
/// `verification.md`, "Groups are not implied" — an environment
/// matrix binds one policy seed to entrypoints that each declare
/// their own, some of them `{ } may_be_empty`). Binding that
/// reference to the policy seed would make the documented shape
/// uncheckable, so `seed-policy` deliberately leaves `guests` open
/// and `app-policy` answers it.
#[test]
fn a_policy_seeds_constitution_still_takes_the_adopters_vocabulary() {
    for seed in ["seed-policy", "app-policy"] {
        let out = check(seed);
        assert!(
            !out.contains("type error"),
            "`{}`: an adopting entrypoint declares the \
             constitution's groups:\n{}",
            seed,
            out
        );
    }
}

/// The importer declares its own `participants` / `rooms` over an
/// innocent locus. The imported claim names the defining seed's
/// declarations, so the violation still fires — a same-named group
/// in the closing seed cannot stand in for it.
#[test]
fn an_importers_same_named_group_is_not_substituted() {
    let out = check("app-bad-collision");
    assert!(
        out.contains("claim `guests_sign_only_via_rooms` violated"),
        "the importer's own `participants` must not satisfy the \
         imported claim:\n{}",
        out
    );
    assert!(
        out.contains("chat::participants"),
        "the violated claim must name the DEFINING seed's group:\n{}",
        out
    );
}

/// The control: the workaround shape stays exactly as valid as it
/// was, directly and through an import.
#[test]
fn the_adopted_constitution_form_stays_green() {
    for seed in ["seed-adopt", "app-adopt"] {
        let out = check(seed);
        assert!(
            !out.contains("type error"),
            "`{}`: the adopted-constitution control must check \
             clean:\n{}",
            seed,
            out
        );
    }
}
