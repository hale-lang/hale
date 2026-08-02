//! User-declared effect classes across a seed boundary (#345).
//!
//! `EffectClass::User(i)` is an index into the DECLARING seed's intern
//! table. Every seed interns from zero, so two seeds that each declare
//! one class both use `User(0)` for different names. Import merging
//! concatenates items, which meant those two classes shared a bit:
//! seed A's `money` and seed B's `pii` became indistinguishable, and a
//! `none: {money}` was checked against whichever one won.
//!
//! v1 avoided that by rejecting cross-seed names outright — sound, but
//! it made a class unusable across the boundary it most wants to
//! cross. The point of `money` is that it holds everywhere the money
//! goes, and in a real codebase the money goes through `lib/`.
//!
//! So the merge remaps instead. This fixture pins BOTH directions,
//! because each alone is passable by a broken implementation:
//!
//!   - the same class one seed away must still fire (a remap that
//!     dropped everything would look "safe" and prove nothing), and
//!   - two DIFFERENT classes that are both index 0 in their own seed
//!     must not alias (the bug the restriction existed to prevent).
//!
//! The second half is the one that caught a real defect: `hale check`
//! on a directory goes through `merge_programs`, which concatenated
//! items while discarding every input's `effect_names`. It reported
//! `quote` reaching `pii` for an assertion that named `money`.

use std::path::PathBuf;
use std::process::Command;

fn check() -> String {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-user-effects/app");
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

#[test]
fn a_user_class_fires_across_a_seed_boundary() {
    let out = check();
    assert!(
        out.contains("effect assertion violated") && out.contains("`quote`"),
        "`quote` reaches `pay::charge`, which declares it carries \
         `money` — the assertion must fire one seed away:\n{}",
        out
    );
    assert!(
        out.contains("money"),
        "the violation must name the class the author declared:\n{}",
        out
    );
}

/// The aliasing case. `money` (app seed) and `pii` (lib seed) are both
/// class 0 where they are declared; only the remap keeps them apart.
#[test]
fn distinct_classes_sharing_a_local_index_do_not_alias() {
    let out = check();
    assert!(
        !out.contains("`label`"),
        "`label` reaches `pii::read_name`, which carries `pii` — NOT \
         the `money` it forbids. Firing here means the two seeds' \
         class 0 aliased onto one bit:\n{}",
        out
    );
    assert!(
        !out.contains("reach `pii`"),
        "no assertion in this fixture names `pii`; reporting it means \
         an index was read against the wrong seed's table:\n{}",
        out
    );
}
