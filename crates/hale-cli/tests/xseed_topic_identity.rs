//! One topic must have one identity across a seed boundary (#334/#332).
//!
//! A qualified topic reference (`relay::Recalled`) kept its qualified
//! form while the declaring seed's own `topic Recalled` was mangled to
//! `__lib_lib_relay_main_Recalled`. Desugaring resolved the two
//! through different paths — the qualified one via
//! `BusSubject::canonical()`, which is SYNTACTIC and returns the last
//! path segment — so one topic became two subjects in the bus graph.
//!
//! Consequences, all from that single split:
//!   - a library locus subscribing to its own topic never received an
//!     importing application's publish (the handler simply never fired)
//!   - the bus graph reported the library's subscription as dead
//!   - qualified topics were invisible to orphan detection, while
//!     unqualified ones in the same shape were reported
//!   - `depends:` could not follow a republisher across a seed
//!
//! The fix canonicalizes qualified topic references in the same pass
//! and against the same rename table that already canonicalized
//! qualified TYPE paths — the bus arm there destructured `{ ty, .. }`
//! and never visited the subject.

use std::path::PathBuf;
use std::process::Command;

fn fixture(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-launderer")
        .join(sub)
}

fn run(cmd: &str, dir: PathBuf) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg(cmd)
        .arg(dir)
        .output()
        .expect("invoke hale");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The whole chain: the app publishes to a topic the LIBRARY declares,
/// the library's own subscriber fires, and its republish reaches back
/// into an app-side subscriber.
#[test]
fn a_library_subscriber_receives_an_applications_publish() {
    let out = run("run", fixture("app"));
    assert!(
        out.contains("final carry = 42"),
        "the laundered value must cross both seed boundaries: {}",
        out
    );
}

/// The graph-level symptom: before the fix the library's subscription
/// was reported dead, because the app's publish landed on a different
/// subject node.
#[test]
fn the_librarys_subscription_is_not_reported_dead() {
    let out = run("check", fixture("app"));
    assert!(
        !out.contains("subscribed but never published"),
        "one topic must be one node in the bus graph: {}",
        out
    );
    assert!(out.contains("typechecked"), "fixture should check: {}", out);
}

/// The `depends:` closure could not cross a seed. It was explicitly a
/// LOWER BOUND when that feature shipped; topic identity closes it,
/// and the diagnostic still names the path through the republisher.
#[test]
fn depends_follows_a_republisher_across_a_seed() {
    // Copy the fixture rather than patching it in place: nextest runs
    // these in parallel, and mutating a shared fixture made the other
    // two tests in this file observe the patched copy.
    let src = fixture("");
    let tmp = std::env::temp_dir().join(format!(
        "hale_xseed_depends_{}_{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_tree(&src, &tmp);

    let app = tmp.join("app/main.hl");
    let original = std::fs::read_to_string(&app).expect("read fixture copy");
    let patched = original.replace(
        "locus StatedCarry {",
        "@effects(depends: {relay::Recalled})\nlocus StatedCarry {",
    );
    assert_ne!(patched, original, "anchor for the depends clause moved");
    std::fs::write(&app, &patched).expect("write fixture copy");

    let out = run("check", tmp.join("app"));
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(
        out.contains("declared dependency set violated"),
        "a dependence laundered through another seed must be caught: {}",
        out
    );
    assert!(
        out.contains("SumLookup") && out.contains("Launderer"),
        "and the path must name the cross-seed republisher: {}",
        out
    );
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for e in std::fs::read_dir(from).expect("read_dir") {
        let e = e.expect("entry");
        let dst = to.join(e.file_name());
        if e.file_type().expect("ft").is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).expect("copy");
        }
    }
}
