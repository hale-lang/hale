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
