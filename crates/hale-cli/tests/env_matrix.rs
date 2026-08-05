//! GH #409, tooling half: `--env` and the (entrypoint × environment)
//! matrix.
//!
//! The property: *any* entrypoint satisfies the claimset for wherever
//! it deploys. That is universal quantification over entrypoints,
//! each still checked independently in its own closed world — it
//! composes nothing and connects nothing.
//!
//! The constitution is bound in `hale.toml` rather than in source
//! because it is a property of the deployment target, not of the
//! program. One entrypoint deployed to two environments must satisfy
//! both claimsets, and it cannot write two conflicting `adopt` lines
//! — but it can be checked twice.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_env_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, src).expect("write");
}

/// STDOUT only. `hale` below folds stderr in so message assertions
/// need not care which stream a diagnostic took — but an artifact
/// must not inherit that, and the program in the provenance test
/// deliberately violates a clause, so its witnesses would land in
/// the JSON.
fn hale_stdout(args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn hale(args: &[&std::ffi::OsStr]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

/// A shared seed declaring the vocabulary AND the constitutions —
/// which is how one law reaches many entrypoints: they all import it.
const LIB: &str = r#"
type Msg { v: Int; }
topic Settled { payload: Msg; subject: "app.settled"; }
locus Billing {
    params { n: Int = 0; }
    bus { publish Settled; }
    fn go() { let m = Msg { v: 1 }; Settled <- m; }
}
locus Research { params { n: Int = 0; } fn look() -> Int { return self.n; } }
group billing = { Billing };
group research = { Research };
constitution Core { tenant_iso: forbid reaches(billing, research); }
constitution Prod extends Core { strict: count publishers(topic Settled) == 7; }
"#;

const APP: &str = r#"
import "../lib" as lb;
main locus A { params { r: lb::Research = lb::Research { }; } }
fn main() { A { }; }
"#;

fn workspace(tag: &str, manifest: &str) -> PathBuf {
    let r = root(tag);
    write(&r, "lib/lib.hl", LIB);
    write(&r, "app-a/main.hl", APP);
    write(&r, "app-b/main.hl", APP);
    write(&r, "hale.toml", manifest);
    r
}

/// The heart of it: one source, two environments, two verdicts.
/// `Core` holds; `Prod` adds a clause that cannot hold here.
#[test]
fn the_same_entrypoint_is_judged_by_where_it_deploys() {
    let r = workspace(
        "twoenv",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\"]\n\
         \n[environments.prod]\nconstitution = \"Prod\"\nentrypoints = [\"app-a\"]\n",
    );
    let app = r.join("app-a");

    let (_, dev) =
        hale(&["check".as_ref(), app.as_os_str(), "--env".as_ref(), "dev".as_ref()]);
    let (out, prod) =
        hale(&["check".as_ref(), app.as_os_str(), "--env".as_ref(), "prod".as_ref()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(dev, 0, "Core holds for this entrypoint");
    assert_ne!(prod, 0, "Prod's extra clause does not: {}", out);
    assert!(
        out.contains("claim `strict` violated"),
        "and the environment's own clause is what fails: {}",
        out
    );
}

/// `extends` reaches through the binding: adopting `Prod` brings
/// `Core`, and the artifact records which constitution each clause
/// came from.
#[test]
fn the_artifact_records_which_constitution_each_clause_came_from() {
    let r = workspace(
        "prov",
        "[environments.prod]\nconstitution = \"Prod\"\nentrypoints = [\"app-a\"]\n",
    );
    let app = r.join("app-a");
    let out = hale_stdout(&[
        "check".as_ref(),
        app.as_os_str(),
        "--env".as_ref(),
        "prod".as_ref(),
        "--dump-topology".as_ref(),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");
    let rows: Vec<(String, String)> = v["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["source"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert!(
        rows.contains(&("tenant_iso".into(), "Core".into())),
        "the inherited clause is attributed to its own origin, not to \
         the adopted constitution: {:?}",
        rows
    );
    assert!(
        rows.contains(&("strict".into(), "Prod".into())),
        "got {:?}",
        rows
    );
}

#[test]
fn the_matrix_checks_every_pair() {
    let r = workspace(
        "matrix",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\", \"app-b\"]\n\
         \n[environments.prod]\nconstitution = \"Core\"\nentrypoints = [\"app-a\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 0, "every pair holds: {}", out);
    assert!(
        out.contains("3 (entrypoint, environment) pair(s) checked"),
        "app-a twice and app-b once: {}",
        out
    );
}

/// The hole composition cannot close by construction: an entrypoint
/// nobody listed is checked against no claimset at all. Silently
/// unconstrained is the failure this feature exists to remove, so it
/// is an error rather than a skip.
#[test]
fn an_entrypoint_in_no_environment_is_an_error() {
    let r = workspace(
        "unbound",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "app-b is bound to nothing: {}", out);
    assert!(
        out.contains("app-b") && out.contains("in no environment"),
        "and it must be named: {}",
        out
    );
}

/// A seed with no `main locus` is not an entrypoint and must not be
/// demanded of the manifest — otherwise every library in the tree
/// would have to be listed.
#[test]
fn a_library_seed_is_not_an_entrypoint() {
    let r = workspace(
        "libonly",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 0, "`lib` declares no main: {}", out);
    assert!(!out.contains("lib is in no environment"), "{}", out);
}

#[test]
fn a_listed_entrypoint_that_does_not_exist_is_reported() {
    let r = workspace(
        "missing",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\", \"app-b\", \"ghost\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(out.contains("does not exist"), "{}", out);
}

#[test]
fn an_unknown_environment_name_is_a_usage_error() {
    let r = workspace(
        "unknownenv",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let app = r.join("app-a");
    let (out, code) = hale(&[
        "check".as_ref(),
        app.as_os_str(),
        "--env".as_ref(),
        "staging".as_ref(),
    ]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{}", out);
    assert!(
        out.contains("declared: dev"),
        "the error should list what IS declared: {}",
        out
    );
}

#[test]
fn env_requires_a_value() {
    let r = workspace(
        "noval",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let app = r.join("app-a");
    let (out, code) =
        hale(&["check".as_ref(), app.as_os_str(), "--env".as_ref()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{}", out);
    assert!(out.contains("--env requires a value"), "{}", out);
}

/// `--matrix` without any `[environments]` has nothing to check, and
/// reporting success would be the fail-open shape again.
#[test]
fn a_manifest_with_no_environments_is_a_usage_error() {
    let r = workspace("noenv", "[deps]\n");
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{}", out);
    assert!(out.contains("[environments."), "{}", out);
}

// =====================================================================
// PR #415 review findings
// =====================================================================

/// Finding 2. `constituton = "Prod"` parsed fine, left `constitution`
/// as `None`, and the entrypoint still counted as bound — a typo
/// silently removing all environment law from a deployment that
/// reported success.
#[test]
fn a_misspelled_manifest_field_is_rejected() {
    let r = workspace(
        "typo",
        "[environments.dev]\nconstituton = \"Core\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "a typo must not be silently ignored: {}", out);
}

/// "This environment adds no law" is a real configuration, but it has
/// to be stated. An omission is indistinguishable from a mistake.
#[test]
fn an_omitted_constitution_must_be_explicit() {
    let r = workspace(
        "omitted",
        "[environments.dev]\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    assert_eq!(code, 2, "{}", out);
    assert!(out.contains("source_only = true"), "name the fix: {}", out);
    let _ = std::fs::remove_dir_all(&r);

    // …and saying so explicitly works.
    let r2 = workspace(
        "sourceonly",
        "[environments.dev]\nsource_only = true\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let (out2, code2) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r2.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r2);
    assert_eq!(code2, 0, "{}", out2);
}

#[test]
fn constitution_and_source_only_are_mutually_exclusive() {
    let r = workspace(
        "both",
        "[environments.dev]\nconstitution = \"Core\"\nsource_only = true\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{}", out);
    assert!(out.contains("one or the other"), "{}", out);
}

/// Finding 4. Without a base, the manifest can bind unrelated
/// constitutions to dev and prod, and "environments may add law,
/// never drop it" is a documentation promise nothing enforces.
/// `[claims] base` makes it true by construction.
#[test]
fn the_workspace_base_rides_along_with_every_environment() {
    let r = workspace(
        "base",
        "[claims]\nbase = \"Core\"\n\n\
         [environments.dev]\nsource_only = true\nentrypoints = [\"app-a\", \"app-b\"]\n\n\
         [environments.prod]\nconstitution = \"Prod\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let app = r.join("app-a");

    let dev = hale_stdout(&[
        "check".as_ref(), app.as_os_str(),
        "--env".as_ref(), "dev".as_ref(),
        "--dump-topology".as_ref(),
    ]);
    let prod = hale_stdout(&[
        "check".as_ref(), app.as_os_str(),
        "--env".as_ref(), "prod".as_ref(),
        "--dump-topology".as_ref(),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    let names = |s: &str| -> Vec<String> {
        let v: serde_json::Value =
            serde_json::from_str(s).expect("artifact parses");
        v["evaluation"]["constitutions"]
            .as_array()
            .expect("constitutions")
            .iter()
            .map(|c| c["name"].as_str().unwrap_or("").to_string())
            .collect()
    };
    assert!(
        names(&dev).contains(&"Core".to_string()),
        "an environment adding nothing still carries the base: {:?}",
        names(&dev)
    );
    let p = names(&prod);
    assert!(
        p.contains(&"Core".to_string()) && p.contains(&"Prod".to_string()),
        "prod ADDS to the base rather than replacing it: {:?}",
        p
    );
}

/// Finding 5. Per-claim `source` says where a clause came from; it
/// cannot say which deployment the run certified. Two environments
/// over one base can produce identical claim rows.
#[test]
fn the_artifact_names_the_constitutions_it_evaluated() {
    let r = workspace(
        "eval",
        "[environments.prod]\nconstitution = \"Prod\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    let app = r.join("app-a");
    let out = hale_stdout(&[
        "check".as_ref(), app.as_os_str(),
        "--env".as_ref(), "prod".as_ref(),
        "--dump-topology".as_ref(),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    let v: serde_json::Value =
        serde_json::from_str(&out).expect("artifact parses");
    let cs = v["evaluation"]["constitutions"]
        .as_array()
        .expect("evaluation.constitutions");
    assert!(
        cs.iter().any(|c| c["name"] == "Prod")
            && cs.iter().any(|c| c["name"] == "Core"),
        "the adopted closure, not just the directly-named one: {}",
        out
    );
    assert!(
        cs.iter().all(|c| c["digest"].as_str().is_some_and(|d| d.len() == 16)),
        "each carries a closure digest: {}",
        out
    );
}

/// Finding 6. Treating an unparseable seed as "not a main" let a
/// syntax error erase an entrypoint from coverage: the matrix
/// reported `ok`, exit 0, while the same seed made valid was
/// correctly flagged as unbound. Breaking a file became a way out of
/// the gate.
#[test]
fn a_malformed_seed_cannot_vanish_from_coverage() {
    let r = workspace(
        "malformed",
        "[environments.dev]\nsource_only = true\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );
    // A main locus missing its closing brace.
    write(&r, "broken/main.hl", "main locus C { params { n: Int = 0; }\nfn main() { C { }; }\n");

    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "an unparseable seed must fail the run: {}", out);
    assert!(
        out.contains("does not parse"),
        "and say why it cannot be shown to be covered: {}",
        out
    );
}

/// Finding 3, at the level it matters: two entrypoints in ONE
/// environment resolving `Core` to different declarations. Binding a
/// bare name proves only that each had *some* constitution called
/// `Core` — the digest is what proves they had the same one.
#[test]
fn one_environment_may_not_mean_two_different_claimsets() {
    let r = root("identity");
    write(&r, "p1/p.hl", LIB);
    // Same display name, one extra clause: a different claimset.
    write(
        &r,
        "p2/p.hl",
        &LIB.replace(
            "constitution Core { tenant_iso: forbid reaches(billing, research); }",
            "constitution Core { tenant_iso: forbid reaches(billing, research); \
             extra: count subscribers(topic Settled) == 0; }",
        ),
    );
    write(&r, "app-a/main.hl", &APP.replace("../lib", "../p1"));
    write(&r, "app-b/main.hl", &APP.replace("../lib", "../p2"));
    write(
        &r,
        "hale.toml",
        "[environments.prod]\nconstitution = \"Core\"\nentrypoints = [\"app-a\", \"app-b\"]\n",
    );

    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    assert_ne!(code, 0, "two different `Core`s must not pass: {}", out);
    assert!(
        out.contains("two different claimsets"),
        "and the failure must say so: {}",
        out
    );

    // Control: both importing the SAME policy seed passes, so the
    // check is not simply rejecting every multi-entrypoint matrix.
    write(&r, "app-b/main.hl", &APP.replace("../lib", "../p1"));
    let (out2, code2) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code2, 0, "one shared declaration is fine: {}", out2);
}

/// Review acceptance 12: an entrypoint that genuinely lacks a
/// component declares the vocabulary explicitly. An undeclared group
/// is an unknown-name error, not an empty set — the PR's original
/// documentation had this backwards.
#[test]
fn an_entrypoint_without_a_component_declares_an_empty_group() {
    let r = root("emptygroup");
    write(
        &r,
        "lib/lib.hl",
        "type Msg { v: Int; }\n\
         topic Settled { payload: Msg; subject: \"app.settled\"; }\n\
         locus Research { params { n: Int = 0; } fn look() -> Int { return self.n; } }\n\
         group research = { Research };\n\
         constitution Core { iso: forbid reaches(billing, research); }\n",
    );
    // This entrypoint has no Billing at all. The group must still be
    // DECLARED — empty and explicitly so.
    write(
        &r,
        "app-a/main.hl",
        "import \"../lib\" as lb;\n\
         group billing = { } may_be_empty;\n\
         main locus A { params { r: lb::Research = lb::Research { }; } }\n\
         fn main() { A { }; }\n",
    );
    write(
        &r,
        "hale.toml",
        "[environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app-a\"]\n",
    );
    let (out, code) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);

    // …and omitting the declaration is an error, not an empty set.
    write(
        &r,
        "app-a/main.hl",
        "import \"../lib\" as lb;\n\
         main locus A { params { r: lb::Research = lb::Research { }; } }\n\
         fn main() { A { }; }\n",
    );
    let (out2, code2) =
        hale(&["check".as_ref(), "--matrix".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 0, "an explicit empty group satisfies it: {}", out);
    assert_ne!(code2, 0, "omitting the declaration must NOT: {}", out2);
    assert!(
        out2.contains("never declared"),
        "with the unknown-name error, not a silent empty set: {}",
        out2
    );
}
