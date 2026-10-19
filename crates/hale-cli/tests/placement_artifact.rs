//! GH #1327: the placement law survives artifact admission with both
//! group identities intact. Admission checks the emitted account; it
//! does not rebuild instance placement or run a second law evaluator.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

#[allow(dead_code)]
#[path = "../src/topology_law.rs"]
mod topology_law;

const SEPARATE: &str = r#"
locus Left { }
locus Right { }
group left = { Left };
group right = { Right };
main locus App {
    params { left: Left = Left { }; right: Right = Right { }; }
    placement { left: pinned; right: pinned; }
    claims { separated: require separate_domains(all left, all right); }
}
fn main() { App { }; }
"#;

fn artifact(tag: &str, source: &str) -> (PathBuf, Value) {
    let dir = std::env::temp_dir().join(format!(
        "hale_placement_artifact_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("app.hl");
    let artifact_path = dir.join("app.topology");
    std::fs::write(&source_path, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&source_path)
        .arg(format!("--dump-topology={}", artifact_path.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = serde_json::from_str(&std::fs::read_to_string(&artifact_path).unwrap()).unwrap();
    (artifact_path, v)
}

fn restamp_law(v: &mut Value) {
    let body = serde_json::to_string(&json!({
        "issues": v["law"]["issues"], "rows": v["law"]["rows"],
    }))
    .unwrap();
    v["law"]["law_digest"] = json!(format!(
        "{:016x}",
        hale_graph::identity::fnv64(body.as_bytes())
    ));
}

fn refused(mut v: Value, message: &str) {
    // Exercise the deep law binding, not merely its integrity digest.
    restamp_law(&mut v);
    let error = topology_law::validate_law_account(&v, "placement").unwrap_err();
    assert!(error.contains(message), "expected {message:?}: {error}");
}

#[test]
fn placement_artifact_round_trips_with_both_groups() {
    let (path, v) = artifact("round_trip", SEPARATE);
    assert_eq!(v["schema"], "1.21");
    assert_eq!(v["adequacy"].as_object().unwrap().len(), 9);
    assert!(v["adequacy"].get("placement").is_some());
    let row = &v["law"]["rows"][0];
    assert_eq!(row["family"], "placement");
    assert_eq!(row["verdict"], "holds");
    assert_eq!(row["law"]["kind"], "require_separate_domains");
    assert_eq!(row["law"]["left"]["display"], "left");
    assert_eq!(row["law"]["right"]["display"], "right");
    assert_eq!(
        v["claims"][0]["form"],
        "require separate_domains(all left, all right)"
    );
    topology_law::validate_law_account(&v, "placement").unwrap();
    let rendered = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["topology", "graph"])
        .arg(path)
        .args(["--format", "mermaid"])
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
}

#[test]
fn placement_admission_rejects_missing_and_extra_payload_fields() {
    let (_, v) = artifact("payload_fields", SEPARATE);
    let mut missing = v.clone();
    missing["law"]["rows"][0]["law"]
        .as_object_mut()
        .unwrap()
        .remove("right");
    refused(missing, "missing field `right`");
    let mut extra = v;
    extra["law"]["rows"][0]["law"]["domain"] = json!("main");
    refused(extra, "unknown field `domain`");
}

#[test]
fn placement_admission_binds_reference_resolution_family_and_form() {
    let (_, v) = artifact("bindings", SEPARATE);
    let mut absent = v.clone();
    absent["law"]["rows"][0]["law"]["right"]["name"] = json!("missing");
    absent["law"]["rows"][0]["law"]["right"]["display"] = json!("missing");
    refused(absent, "does not match any canonical pair");
    let mut unresolved = v.clone();
    unresolved["law"]["rows"][0]["law"]["right"]["resolved"] = json!(false);
    refused(unresolved, "only `invalid` is truthful");
    let mut family = v.clone();
    family["law"]["rows"][0]["family"] = json!("endpoint");
    refused(family, "kind belongs to `placement`");
    let mut substituted = v.clone();
    substituted["law"]["rows"][0]["law"]["right"] = v["law"]["rows"][0]["law"]["left"].clone();
    refused(substituted, "form does not render from the typed law");
    let mut no_form = v;
    no_form["claims"][0].as_object_mut().unwrap().remove("form");
    refused(no_form, "form does not render from the typed law");
}

#[test]
fn placement_admission_keeps_evidence_and_claim_projection_obligations() {
    let (_, v) = artifact("accountability", SEPARATE);
    for verdict in ["violated", "uncertified"] {
        let mut bare = v.clone();
        bare["law"]["rows"][0]["verdict"] = json!(verdict);
        refused(bare, "retains none of its judgment's evidence");
    }
    let mut hidden = v.clone();
    hidden["claims"] = json!([]);
    refused(
        hidden,
        "claims rows and the claims-tier law rows are not one-to-one",
    );
    let mut duplicate = v;
    let mut law = duplicate["law"]["rows"][0].clone();
    law["ordinal"] = json!(1);
    duplicate["law"]["rows"].as_array_mut().unwrap().push(law);
    let mut claim = duplicate["claims"][0].clone();
    claim["ordinal"] = json!(1);
    duplicate["claims"].as_array_mut().unwrap().push(claim);
    refused(duplicate, "duplicate claim name `separated`");
}

#[test]
fn placement_requires_the_exact_schema_adequacy_account() {
    let (_, v) = artifact("adequacy", SEPARATE);
    let mut absent = v.clone();
    absent["adequacy"]
        .as_object_mut()
        .unwrap()
        .remove("placement");
    refused(absent, "adequacy must carry exactly");
    let mut extra = v.clone();
    extra["adequacy"]["invented"] = json!("exact");
    refused(extra, "adequacy must carry exactly");
    let mut wrong = v.clone();
    wrong["adequacy"]["placement"] = if v["adequacy"]["placement"] == "exact" {
        json!("degraded")
    } else {
        json!("exact")
    };
    refused(wrong, "adequacy.placement disagrees");
    for schema in ["1.14", "1.15", "1.16", "1.17", "1.18", "1.19"] {
        let mut old = v.clone();
        old["schema"] = json!(schema);
        old["adequacy"].as_object_mut().unwrap().remove("placement");
        refused(old, "requires schema 1.20");
    }
}

#[test]
fn historical_schemas_keep_their_existing_family_accounts() {
    // The law decoder also serves callers that validate older account
    // layouts directly. Keep each historical family's exact key set.
    let source = SEPARATE.replace(
        "claims { separated: require separate_domains(all left, all right); }",
        "",
    );
    let (_, v) = artifact("historical", &source);
    for (schema, removed) in [
        ("1.14", &["placement", "budget", "depends", "causes"][..]),
        ("1.15", &["placement", "budget", "depends"][..]),
        ("1.16", &["placement", "budget"][..]),
        ("1.17", &["placement"][..]),
        ("1.18", &["placement"][..]),
        ("1.19", &["placement"][..]),
    ] {
        let mut old = v.clone();
        old["schema"] = json!(schema);
        for family in removed {
            old["adequacy"].as_object_mut().unwrap().remove(*family);
        }
        topology_law::validate_law_account(&old, schema).unwrap();
        old["adequacy"]["placement"] = json!("exact");
        refused(old, "adequacy must carry exactly");
    }
}
