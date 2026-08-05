//! The topology artifact's external contract: it must be valid JSON,
//! and observing a program must not change what checking it MEANS.
//!
//! Both properties were broken in v0.15.0 and both were reported from
//! outside (claims developer-experience review):
//!
//!  - **Malformed JSON.** The `claims` array was never closed before
//!    `"lowered"` began, so *every* artifact — no claims, one, many,
//!    with or without lowered rows — was rejected by any standards-
//!    compliant parser. It survived because the existing artifact
//!    tests assert on substrings and extract the `shape_hash` line;
//!    none parsed the whole document. `artifact_is_valid_json` does,
//!    which is the only shape of test that could have caught it.
//!  - **Dump mode returned success.** `hale check failing.hl
//!    --dump-topology` exited 0 with no diagnostics, while the same
//!    file without the flag exited 1 with its witness. A CI job that
//!    added the flag to collect an artifact silently stopped gating.
//!
//! The flag-parsing tests cover the third finding: `--flag=value`
//! was silently ignored and the command SUCCEEDED, which for a CI
//! gate is the worst failure mode — green while gating nothing.

use std::process::Command;

fn write_tmp(tag: &str, src: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hale_topo_contract_{}_{}.hl",
        std::process::id(),
        tag
    ));
    std::fs::write(&path, src).expect("write program");
    path
}

fn hale(args: &[&std::ffi::OsStr]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// A program whose claim holds.
const PASSING: &str = r#"
    type T { v: Int; }
    topic Tk { payload: T; subject: "app.t"; }
    locus Sub {
        params { got: Int = 0; }
        bus { subscribe Tk as on_t; }
        fn on_t(t: T) { self.got = t.v; }
    }
    group subs = { Sub };
    main locus App {
        params { s: Sub = Sub { }; }
        claims { wired: require subscribes(some subs, topic Tk); }
    }
    fn main() { App { }; }
"#;

/// A program whose claim is violated: nothing subscribes the topic.
const FAILING: &str = r#"
    type T { v: Int; }
    topic Tk { payload: T; subject: "app.t"; }
    locus Quiet { params { n: Int = 0; } fn go() -> Int { return self.n; } }
    group subs = { Quiet };
    main locus App {
        params { q: Quiet = Quiet { }; }
        claims { wired: require subscribes(some subs, topic Tk); }
    }
    fn main() { App { }; }
"#;

/// Parse the WHOLE artifact. Substring assertions cannot catch a
/// structural error, which is exactly how the unclosed array shipped.
#[test]
fn artifact_is_valid_json() {
    for (tag, src) in [("passing", PASSING), ("failing", FAILING)] {
        let path = write_tmp(tag, src);
        let (stdout, _) =
            hale(&["check".as_ref(), path.as_os_str(), "--dump-topology".as_ref()]);
        let _ = std::fs::remove_file(&path);
        let v: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| {
                panic!("artifact ({tag}) is not valid JSON: {e}\n{stdout}")
            });
        // and it is the documented shape, not merely parseable
        for key in
            ["schema", "shape_hash", "sorts", "relations", "claims", "lowered"]
        {
            assert!(
                v.get(key).is_some(),
                "artifact ({tag}) missing `{key}`: {stdout}"
            );
        }
        assert!(
            v["claims"].is_array() && v["lowered"].is_array(),
            "claims/lowered must be arrays ({tag})"
        );
    }
}

/// Observing a program must not change its verdict. Dump mode used to
/// return SUCCESS before the checker ran.
#[test]
fn dump_mode_preserves_the_exit_status() {
    let fail = write_tmp("exit_fail", FAILING);
    let pass = write_tmp("exit_pass", PASSING);

    let (_, plain) = hale(&["check".as_ref(), fail.as_os_str()]);
    let (dumped, with_flag) =
        hale(&["check".as_ref(), fail.as_os_str(), "--dump-topology".as_ref()]);
    let (_, pass_code) =
        hale(&["check".as_ref(), pass.as_os_str(), "--dump-topology".as_ref()]);

    let _ = std::fs::remove_file(&fail);
    let _ = std::fs::remove_file(&pass);

    assert_ne!(plain, 0, "the failing program must fail without the flag");
    assert_eq!(
        with_flag, plain,
        "adding --dump-topology must not change the verdict — a CI job \
         that adds it to collect an artifact would silently stop gating"
    );
    assert!(
        !dumped.is_empty(),
        "the artifact must still be emitted for a failing program"
    );
    assert_eq!(pass_code, 0, "a passing program still exits 0");
}

/// `--flag=value` was silently ignored AND the command succeeded.
#[test]
fn equals_spelling_is_honored_for_the_baseline_gate() {
    let path = write_tmp("eq", PASSING);
    let base = std::env::temp_dir()
        .join(format!("hale_topo_base_{}.json", std::process::id()));

    let (artifact, _) =
        hale(&["check".as_ref(), path.as_os_str(), "--dump-topology".as_ref()]);
    std::fs::write(&base, &artifact).expect("write baseline");

    let eq_arg = format!("--check-topology={}", base.display());
    let (_, matching) =
        hale(&["check".as_ref(), path.as_os_str(), eq_arg.as_ref()]);
    assert_eq!(matching, 0, "a matching baseline passes");

    // Perturb the baseline: the gate must now fail. Before the fix
    // the `=` spelling was ignored, so this returned 0 — green while
    // gating nothing.
    std::fs::write(&base, artifact.replace("\"wired\"", "\"renamed\""))
        .expect("rewrite baseline");
    let (_, mismatched) =
        hale(&["check".as_ref(), path.as_os_str(), eq_arg.as_ref()]);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&base);
    assert_ne!(
        mismatched, 0,
        "a mismatched baseline must fail through the `=` spelling too"
    );
}

/// A missing operand was silently ignored and the command succeeded.
#[test]
fn a_missing_flag_operand_is_a_usage_error() {
    let path = write_tmp("noarg", PASSING);
    let (_, code) =
        hale(&["check".as_ref(), path.as_os_str(), "--check-topology".as_ref()]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        code, 2,
        "`--check-topology` with no path must be a usage error, not a \
         silent no-op that reports success"
    );
}

/// `--dump-topology <path>` used to ignore the path, write to stdout,
/// and create no file. The `=` spelling now writes the artifact.
#[test]
fn dump_topology_can_write_to_a_file() {
    let path = write_tmp("tofile", PASSING);
    let out = std::env::temp_dir()
        .join(format!("hale_topo_out_{}.json", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let arg = format!("--dump-topology={}", out.display());
    let (_, code) = hale(&["check".as_ref(), path.as_os_str(), arg.as_ref()]);
    assert_eq!(code, 0, "passing program exits 0");

    let written = std::fs::read_to_string(&out).expect("artifact file written");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out);
    let v: serde_json::Value =
        serde_json::from_str(&written).expect("file holds valid JSON");
    assert!(v.get("shape_hash").is_some());
}

/// P2 from the devex review: `--check-topology` compares the entire
/// artifact, so a comment-only edit that shifts every provenance
/// offset failed the gate while reporting that the *model* had
/// changed — when `shape_hash`, the model's identity, had not.
///
/// Both gates now exist and are named for what they compare. This
/// pins the distinction in both directions, which is the only way to
/// show the loose gate is loose without being useless.
#[test]
fn the_shape_gate_ignores_source_motion_but_not_graph_changes() {
    let path = write_tmp("shape", PASSING);
    let base = std::env::temp_dir()
        .join(format!("hale_topo_shape_{}.json", std::process::id()));
    let (artifact, _) =
        hale(&["check".as_ref(), path.as_os_str(), "--dump-topology".as_ref()]);
    std::fs::write(&base, &artifact).expect("write baseline");

    // (a) a leading comment shifts every span; the model is identical
    let moved = write_tmp(
        "shape_moved",
        &format!("// leading comment that shifts every span\n{PASSING}"),
    );
    let exact = format!("--check-topology={}", base.display());
    let shape = format!("--check-topology-shape={}", base.display());

    let (_, exact_code) =
        hale(&["check".as_ref(), moved.as_os_str(), exact.as_ref()]);
    let (_, shape_code) =
        hale(&["check".as_ref(), moved.as_os_str(), shape.as_ref()]);
    assert_ne!(
        exact_code, 0,
        "the exact gate is a full snapshot and DOES see provenance motion"
    );
    assert_eq!(
        shape_code, 0,
        "the shape gate must not fire on source motion — that churn is \
         what made the single gate impractical"
    );

    // (b) a real graph change must still trip it
    let grown = write_tmp(
        "shape_grown",
        &PASSING.replace(
            "locus Sub {",
            "locus Extra { params { n: Int = 0; } }\n    locus Sub {",
        ),
    );
    let (_, grown_code) =
        hale(&["check".as_ref(), grown.as_os_str(), shape.as_ref()]);

    for p in [&path, &moved, &grown] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(&base);
    assert_ne!(
        grown_code, 0,
        "adding a locus changes the model and MUST trip the shape gate"
    );
}

/// A program that does not TYPECHECK must not produce an artifact at
/// all. This is the other half of "observing a program must not
/// change its verdict", and it fails in the more dangerous direction.
///
/// Verified against the shipped behavior before the fix: a program
/// with a type error emitted a full artifact — populated relations,
/// and claims evaluated over a graph derived from source the compiler
/// could not understand. A claim reported `"result": "holds"` for a
/// program that cannot compile. That is worse for a consumer than no
/// artifact, because an admission step looking for "no violated
/// claims" passes it: there are none.
///
/// A VIOLATED claim is the opposite case and must still emit — the
/// model is well-defined, the row is truthful, and replaying a
/// violation independently is the point of publishing the model.
#[test]
fn a_program_that_does_not_typecheck_emits_no_artifact() {
    const TYPE_ERROR: &str = r#"
        type T { v: Int; }
        topic Tk { payload: T; subject: "app.t"; }
        locus Sub {
            params { got: Int = 0; }
            bus { subscribe Tk as on_t; }
            fn on_t(t: T) { self.got = t.v; }
            fn broken() -> Int { return self.no_such_field; }
        }
        group subs = { Sub };
        main locus App {
            params { s: Sub = Sub { }; }
            claims { wired: require subscribes(some subs, topic Tk); }
        }
        fn main() { App { }; }
    "#;
    let path = write_tmp("typeerr", TYPE_ERROR);
    let (stdout, code) =
        hale(&["check".as_ref(), path.as_os_str(), "--dump-topology".as_ref()]);
    let _ = std::fs::remove_file(&path);

    assert!(
        stdout.trim().is_empty(),
        "no artifact may be emitted for a program that does not \
         typecheck — its model describes no program: {}",
        stdout
    );
    assert_ne!(code, 0, "the type error must still fail the run");
}

/// The distinction the fix rests on: a violated claim still emits.
#[test]
fn a_violated_claim_still_emits_its_artifact() {
    let path = write_tmp("violated_emits", FAILING);
    let (stdout, code) =
        hale(&["check".as_ref(), path.as_os_str(), "--dump-topology".as_ref()]);
    let _ = std::fs::remove_file(&path);

    assert_ne!(code, 0, "the violated claim must fail the run");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("a claims-violated program must still emit a parseable artifact: {e}\n{stdout}")
    });
    assert!(
        v["claims"].as_array().is_some_and(|c| !c.is_empty()),
        "and it must carry the claim rows that explain the failure: {}",
        stdout
    );
}

/// A violated **fn-grained certificate** is a law failure, not a type
/// error, so the artifact must still be emitted — the `lowered` rows
/// exist precisely to record that verdict.
///
/// This is the case CI caught and local runs did not. The soundness
/// gate first keyed on `DiagKind::Claim`, which only bundle claims
/// carried, so a program that typechecked but broke a `@budget` or
/// `@effects` contract was refused an artifact. Its model is
/// perfectly sound; only a rule was broken.
#[test]
fn a_violated_certificate_still_emits_its_artifact() {
    const OVER_BUDGET: &str = r#"
        type P { a: Int; }
        @budget(alloc_per_call = 0)
        fn boxed(n: Int) -> P { return P { a: n }; }
        main locus App { params { n: Int = 0; } }
        fn main() { App { }; let p = boxed(1); }
    "#;
    let path = write_tmp("certificate", OVER_BUDGET);
    let (stdout, code) =
        hale(&["check".as_ref(), path.as_os_str(), "--dump-topology".as_ref()]);
    let _ = std::fs::remove_file(&path);

    assert_ne!(code, 0, "the broken contract must fail the run: {}", stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "a violated CERTIFICATE is law, not a broken model — the \
             artifact must still be emitted: {e}\n{stdout}"
        )
    });
    assert!(
        v["lowered"].as_array().is_some_and(|r| !r.is_empty()),
        "and carry the lowered rows that record the verdict: {}",
        stdout
    );
    assert_eq!(
        v["verdict"], "law_failed",
        "the document verdict covers certificates too: {}",
        stdout
    );
}
