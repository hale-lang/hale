//! GH #382 phases 2 + 5 across a seed boundary: qualified topic
//! references in claims (`topic t::Tasks`), `cover` over an
//! imported vocabulary seed, and the topology artifact round-trip.
//!
//! Qualified refs canonicalize at the mangle stage — the same #334
//! path bus subjects take — never by name-suffix matching, and the
//! artifact + diagnostics demangle back to author spelling.

use std::path::PathBuf;
use std::process::Command;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-claims-verbs/app")
}

fn check(extra: &[&str]) -> (String, bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hale"));
    cmd.arg("check").arg(fixture());
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd.output().expect("run hale check");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

/// `require` + `count` hold through qualified refs; `cover` catches
/// the orphan topic — and names it in author spelling.
#[test]
fn cover_catches_an_uncovered_topic_across_seeds() {
    let (out, ok) = check(&[]);
    assert!(!ok, "the uncovered topic must fail the build:\n{}", out);
    assert!(
        out.contains("claim `no_orphans` violated")
            && out.contains("t::Digest_t"),
        "the violation must name the uncovered topic in author \
         spelling:\n{}",
        out
    );
    assert!(
        !out.contains("claim `wired`") && !out.contains("claim `single`"),
        "the wired require and the single-writer count must hold:\n{}",
        out
    );
    assert!(
        !out.contains("__lib_"),
        "no mangled symbol may leak:\n{}",
        out
    );
}

/// The topology artifact: schema + shape_hash + demangled model +
/// per-claim results, and the `--check-topology` round-trip (same
/// artifact passes, a stale artifact fails with a diff).
#[test]
fn the_topology_artifact_round_trips() {
    let (dump, _ok) = check(&["--dump-topology"]);
    assert!(
        dump.contains("\"schema\": \"1.0\"")
            && dump.contains("\"shape_hash\": \""),
        "the artifact must carry schema + shape_hash:\n{}",
        dump
    );
    assert!(
        dump.contains("\"subject\": \"t::Tasks\"")
            || dump.contains("{\"subject\": \"Tasks\""),
        "the model must carry the subscribes relation:\n{}",
        dump
    );
    assert!(
        dump.contains(
            "{\"name\": \"no_orphans\", \"form\": \"cover topic in \
             seed(t): subscribed_by(some staff)\", \"result\": \
             \"violated\"}"
        ),
        "the artifact must record the violated claim:\n{}",
        dump
    );
    assert!(
        dump.contains("\"result\": \"holds\""),
        "the artifact must record the holding claims too:\n{}",
        dump
    );
    assert!(
        !dump.contains("__lib_"),
        "the artifact must be in author spelling:\n{}",
        dump
    );

    // Round-trip: the emitted artifact IS the baseline.
    let tmp = std::env::temp_dir().join(format!(
        "hale_topology_rt_{}.json",
        std::process::id()
    ));
    std::fs::write(&tmp, &dump).expect("write baseline");
    let (out, ok) =
        check(&["--check-topology", tmp.to_str().unwrap()]);
    // The claims themselves still fail the build (no_orphans), but
    // the topology gate must not add a mismatch complaint.
    assert!(
        !out.contains("topology changed"),
        "a freshly-dumped artifact must match:\n{}",
        out
    );
    let _ = ok;

    // A stale baseline fails with the regenerate hint.
    std::fs::write(&tmp, dump.replace("t::Tasks", "t::Renamed"))
        .expect("write stale baseline");
    let (out, ok) =
        check(&["--check-topology", tmp.to_str().unwrap()]);
    assert!(
        !ok && out.contains("topology changed")
            && out.contains("--dump-topology"),
        "a stale artifact must fail with the regenerate hint:\n{}",
        out
    );
    let _ = std::fs::remove_file(&tmp);
}

/// shape_hash identifies the MODEL half: same fixture, two dumps,
/// one hash.
#[test]
fn shape_hash_is_stable_across_dumps() {
    let (a, _) = check(&["--dump-topology"]);
    let (b, _) = check(&["--dump-topology"]);
    assert_eq!(a, b, "two dumps of one bundle must be identical");
}
