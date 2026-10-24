//! GH #1327: the placement law survives artifact admission with both
//! group identities intact. Admission checks the emitted account; it
//! does not rebuild instance placement or run a second law evaluator.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

#[allow(dead_code)]
#[path = "../src/topology_law.rs"]
mod topology_law;

#[allow(dead_code)]
#[path = "../src/fleet_model.rs"]
mod fleet_model;

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
    artifact_with_outcome(tag, source, true)
}

fn artifact_with_outcome(tag: &str, source: &str, succeeds: bool) -> (PathBuf, Value) {
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
    assert_eq!(
        out.status.success(), succeeds,
        "{}", String::from_utf8_lossy(&out.stderr)
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

/// The historical tests exercise the older law-account layout directly,
/// without whole-document hash admission. Remove the new executable-default
/// vocabulary as well as changing the schema; relabeling a current document
/// alone must be refused (tested below).
fn historical_account(v: &Value, schema: &str) -> Value {
    let mut old = v.clone();
    old["schema"] = json!(schema);
    let defaults: std::collections::BTreeSet<String> = old["law"]["fn_universe"].as_array().unwrap().iter()
        .filter(|r| matches!(r["kind"].as_str(), Some("parameter_default" | "field_default"))).map(|r| r["display"].as_str().unwrap().to_string()).collect();
    old["law"]["fn_universe"].as_array_mut().unwrap().retain(|r| !defaults.contains(r["display"].as_str().unwrap()));
    old["sorts"]["fns"].as_array_mut().unwrap().retain(|r| !defaults.contains(r.as_str().unwrap()));
    for section in ["effects", "labels"] {
        old[section].as_object_mut().unwrap().retain(|name, _| !defaults.contains(name));
    }
    old["law"].as_object_mut().unwrap().remove("types");
    old["sorts"].as_object_mut().unwrap().remove("types");
    for table in ["initializers", "parameter_defaults", "constructions", "initializes"] {
        old["relations"].as_object_mut().unwrap().remove(table);
    }
    old
}

#[test]
fn placement_artifact_round_trips_with_both_groups() {
    let (path, v) = artifact("round_trip", SEPARATE);
    assert_eq!(v["schema"], hale_types::topology::TOPOLOGY_SCHEMA);
    assert_eq!(v["adequacy"].as_object().unwrap().len(), 11);
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
        let mut old = historical_account(&v, schema);
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
        let mut old = historical_account(&v, schema);
        old["adequacy"].as_object_mut().unwrap().remove("resource");
        old["adequacy"].as_object_mut().unwrap().remove("flow_policy");
        old["capabilities"].as_object_mut().unwrap().remove("exact_flow_classification");
        for family in removed {
            old["adequacy"].as_object_mut().unwrap().remove(*family);
        }
        topology_law::validate_law_account(&old, schema).unwrap();
        old["adequacy"]["placement"] = json!("exact");
        refused(old, "adequacy must carry exactly");
    }
}

const RESERVATIONS: &str = r#"
type Item { n: Int; }
topic Items { payload: Item; }
locus Receiver {
    bus { subscribe Items as receive bounded(8, drop_new); }
    fn receive(item: Item) { }
}
group receivers = { Receiver };
main locus App {
    params { a: Receiver = Receiver { }; b: Receiver = Receiver { }; }
    bus { publish Items; }
    claims { slots: bound queue_slots <= 16 on instances receivers; }
    run() { Items <- Item { n: 1 }; }
}
fn main() { App { }; }
"#;

#[test]
fn resource_artifact_round_trips_and_validates_the_closed_payload() {
    let (path, v) = artifact("resource_round_trip", RESERVATIONS);
    assert_eq!(v["schema"], hale_types::topology::TOPOLOGY_SCHEMA);
    assert!(v["adequacy"].get("resource").is_some());
    assert_eq!(v["law"]["rows"][0]["family"], "resource");
    assert_eq!(v["law"]["rows"][0]["law"]["kind"], "queue_slots");
    assert_eq!(v["law"]["rows"][0]["law"]["limit"], 16);
    assert_eq!(v["claims"][0]["form"], "bound queue_slots <= 16 on instances receivers");
    topology_law::validate_law_account(&v, "resource").unwrap();
    let rendered = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["topology", "graph"]).arg(path).args(["--format", "mermaid"])
        .output().unwrap();
    assert!(rendered.status.success(), "{}", String::from_utf8_lossy(&rendered.stderr));
    let mut negative = v.clone();
    negative["law"]["rows"][0]["law"]["limit"] = json!(-1);
    refused(negative, "nonnegative integer");
    let mut changed = v.clone();
    changed["law"]["rows"][0]["law"]["limit"] = json!(15);
    refused(changed, "form does not render from the typed law");
    let mut missing = v.clone();
    missing["law"]["rows"][0]["law"].as_object_mut().unwrap().remove("group");
    refused(missing, "missing field `group`");
    let mut extra = v.clone();
    extra["law"]["rows"][0]["law"]["bytes"] = json!(128);
    refused(extra, "unknown field `bytes`");
    let (_, legacy) = artifact("resource_without_lifecycle", &RESERVATIONS.replace("run() { Items <- Item { n: 1 }; }", ""));
    for schema in ["1.20", "1.21"] {
        let mut old = historical_account(&legacy, schema);
        old["adequacy"].as_object_mut().unwrap().remove("resource");
        refused(old, "resource laws require schema 1.22");
    }
    for verdict in ["violated", "uncertified"] {
        let mut unsupported = v.clone();
        unsupported["law"]["rows"][0]["verdict"] = json!(verdict);
        refused(unsupported, "retains none of its judgment's evidence");
    }
}

#[test]
fn excess_queue_reservations_fail_both_check_and_verify() {
    let (path, _) = artifact("resource_verbs", RESERVATIONS);
    let source = path.with_file_name("app.hl");
    std::fs::write(&source, RESERVATIONS.replace("<= 16", "<= 15")).unwrap();
    for verb in ["check", "verify"] {
        let result = Command::new(env!("CARGO_BIN_EXE_hale"))
            .arg(verb).arg(&source).output().unwrap();
        assert!(!result.status.success());
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        assert!(diagnostic.contains("16 subscription queue slots"), "{diagnostic}");
    }
}

const FLOW_POLICY: &str = r#"
locus Job { }
locus Owner { accept(k: Job) { } release(k: Job) { } }
group jobs = { Job };
main locus App { claims { reclaimable: require flow(all jobs); } }
fn main() { App { }; }
"#;

#[test]
fn flow_policy_admission_binds_schema_classification_and_evidence() {
    let (path, v) = artifact("flow_policy", FLOW_POLICY);
    assert_eq!(v["schema"], hale_types::topology::TOPOLOGY_SCHEMA);
    assert_eq!(v["law"]["rows"][0]["family"], "flow_policy");
    assert_eq!(v["law"]["rows"][0]["law"]["kind"], "require_flow");
    assert_eq!(v["claims"][0]["form"], "require flow(all jobs)");
    assert_eq!(v["flow_classes"], json!([{"locus":"Job", "class":"flow"}]));
    assert_eq!(v["adequacy"]["flow_policy"], "exact");
    topology_law::validate_law_account(&v, "flow").unwrap();
    let graph = Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology", "graph"]).arg(&path).output().unwrap();
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    let mut changed = v.clone();
    changed["law"]["rows"][0]["law"]["group"]["display"] = json!("other");
    refused(changed, "group");
    let mut missing = v.clone();
    missing["law"]["rows"][0]["law"].as_object_mut().unwrap().remove("group");
    refused(missing, "missing field `group`");
    let old = historical_account(&v, "1.22");
    refused(old, "parent protocol accounts require topology schema 1.35");
    // Exercise the older flow-law gate independently of the new parent
    // source-account requirement, using a resident with no parent callbacks.
    let (_, resident) = artifact_with_outcome("flow_resident_historical", &FLOW_POLICY.replace("locus Owner { accept(k: Job) { } release(k: Job) { } }", ""), false);
    refused(historical_account(&resident, "1.22"), "flow-policy laws require schema 1.23");
    let mut tag = v.clone(); tag["flow_classes"][0]["class"] = json!("resident");
    refused(tag, "flow_classes.class");
    let mut duplicate = v.clone(); duplicate["flow_classes"].as_array_mut().unwrap().push(v["flow_classes"][0].clone());
    refused(duplicate, "sorted and unique");
    let mut unknown = v.clone(); unknown["flow_classes"][0]["locus"] = json!("Missing");
    refused(unknown, "unknown locus");
    let mut template = v.clone(); template["flow_classes"][0]["class"] = json!("template");
    refused(template, "contradicts exact_flow_classification");
    let mut absent = v.clone(); absent["capabilities"].as_object_mut().unwrap().remove("exact_flow_classification");
    refused(absent, "known boolean flags");
    let mut unsupported = v.clone(); unsupported["law"]["rows"][0]["verdict"] = json!("uncertified");
    refused(unsupported, "retains none of its judgment's evidence");
    let mut bad_adequacy = v; bad_adequacy["adequacy"]["flow_policy"] = json!("degraded");
    refused(bad_adequacy, "adequacy.flow_policy disagrees");
}

#[test]
fn recovery_residue_survives_artifact_admission() {
    let src = SEPARATE.replace("locus Left { }", "locus Left { on_failure(c: Right, e: ClosureViolation) { restart(c); } }")
        .replace("    claims { separated: require separate_domains(all left, all right); }", "");
    let (path, v) = artifact("recovery_residue", &src);
    assert_eq!(v["schema"], hale_types::topology::TOPOLOGY_SCHEMA);
    assert_eq!(v["semantics"], hale_types::topology::MODEL_SEMANTICS);
    assert!(v["unknowns"].as_array().unwrap().iter().any(|row| row["reasons"].as_array().is_some_and(|reasons| reasons.iter().any(|r| r == "unmodeled_recovery"))));
    assert_eq!(v["capabilities"]["exact_calls"], false);
    assert_eq!(v["capabilities"]["exact_effects"], false);
    assert_eq!(v["capabilities"]["exact_costs"], false);
    topology_law::validate_law_account(&v, "recovery").unwrap();
    let old = historical_account(&v, "1.23");
    assert!(topology_law::validate_law_account(&old, "old recovery").unwrap_err().contains("marked analyzed"),
        "the expanded handler coverage contract requires schema 1.24");
    let graph = Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology", "graph"]).arg(&path).output().unwrap();
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
}

#[test]
fn recovery_budget_remains_uncertified_through_artifact_admission() {
    let src = SEPARATE.replace("    claims { separated: require separate_domains(all left, all right); }", "")
        .replace("fn main()", "@budget(alloc_per_call = 0) fn recover(c: Right) { restart(c); }\nfn main()");
    let (path, v) = artifact_with_outcome("recovery_budget", &src, false);
    let row = v["law"]["rows"].as_array().unwrap().iter().find(|r| r["family"] == "budget").unwrap();
    assert_eq!(row["verdict"], "uncertified");
    assert_eq!(row["certs"][0]["result"], "uncertified");
    assert_eq!(v["verdict"], "law_failed");
    topology_law::validate_law_account(&v, "recovery budget").unwrap();
    let graph = Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology", "graph"]).arg(&path).output().unwrap();
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
}

#[test]
fn evaluated_recovery_defaults_agree_in_check_verify_and_artifact() {
    let source = r#"
locus Child { }
fn recover(c: Child) { restart(c); }
type Settings { n: Int = if true { recover(Child { }); 0 } else { 0 }; }
fn measure() { let value = Settings { }; }
main locus App { run() { let value = Settings { }; } }
fn main() { App { }; }
"#;
    for (tag, source) in [
        ("default_budget", source.replace("fn measure()", "@budget(publish = 0) fn measure()")),
        ("default_effect", source.replace("fn measure()", "@no_syscall fn measure()")),
        ("default_no_panic", source.replace("fn measure()", "@no_panic fn measure()")),
        ("default_phase", source.replace("main locus App", "@phase_effects(birth: {}, run: {alloc}) main locus App")),
    ] {
        let (path, v) = artifact_with_outcome(tag, &source, false);
        let rows = v["law"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        let expected = if tag == "default_phase" { "violated" } else { "uncertified" };
        assert_eq!(rows[0]["verdict"], expected);
        if tag == "default_phase" {
            let certs = rows[0]["certs"].as_array().unwrap();
            assert_eq!(certs.len(), 2);
            assert_eq!(certs[0]["result"], "holds", "the absent birth body stays exact");
            assert_eq!(certs[1]["result"], "violated", "constructing Child in the default has a known spawn effect outside the run allowance");
        }
        assert_eq!(v["verdict"], "law_failed");
        topology_law::validate_law_account(&v, tag).unwrap();
        let verify = Command::new(env!("CARGO_BIN_EXE_hale")).arg("verify").arg(path.parent().unwrap().join("app.hl")).output().unwrap();
        assert!(!verify.status.success(), "verify cannot certify what check refuses");
        let text = format!("{}{}", String::from_utf8_lossy(&verify.stdout), String::from_utf8_lossy(&verify.stderr));
        assert!(text.contains(expected), "{text}");
        let graph = Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology", "graph"]).arg(&path).output().unwrap();
        assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    }
}

#[test]
fn implicit_failure_dispatch_agrees_in_check_verify_and_artifact() {
    let source = r#"
locus Child {
    params { n: Int = 0; }
    closure broken { captures: n; epoch inline; }
    @effects(none: {stdio}) fn break_it() { self.n = 1; violate broken; }
}

main locus App {
    params { child: Child = Child { }; }
    on_failure(c: Child, e: ClosureViolation) { println("failure handler ran"); }
    run() { self.child.break_it(); }
}
fn main() { App { }; }
"#;
    for (tag, source, expected) in [
        ("failure_effect", source.to_string(), "uncertified"),
        ("failure_budget", source.replace("@effects(none: {stdio})", "@budget(publish = 0)"), "uncertified"),
        ("failure_direct", source.replace("self.n = 1;", "println(1);"), "violated"),
    ] {
        let (path, v) = artifact_with_outcome(tag, &source, false);
        assert_eq!(v["law"]["rows"][0]["verdict"], expected);
        assert_eq!(v["verdict"], "law_failed");
        assert_eq!(v["capabilities"]["exact_publishes"], true, "unknown invocation does not invent a direct publish site");
        assert_eq!(v["capabilities"]["exact_calls"], false);
        assert_eq!(v["capabilities"]["exact_costs"], false);
        assert!(v["unknowns"].as_array().unwrap().iter().any(|row| row["reasons"].as_array().unwrap().iter()
            .any(|reason| reason == "unmodeled_failure_dispatch")));
        topology_law::validate_law_account(&v, tag).unwrap();
        let mut older = v.clone(); older["schema"] = json!("1.24");
        assert!(topology_law::validate_law_account(&older, tag).unwrap_err().contains("requires topology schema 1.25"));
        let verify = Command::new(env!("CARGO_BIN_EXE_hale")).arg("verify").arg(path.parent().unwrap().join("app.hl")).output().unwrap();
        assert!(!verify.status.success());
        let text = format!("{}{}", String::from_utf8_lossy(&verify.stdout), String::from_utf8_lossy(&verify.stderr));
        assert!(text.contains(expected), "{text}");
        let graph = Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology", "graph"]).arg(&path).output().unwrap();
        assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    }
}

const INITIALIZATION: &str = r#"
fn observe() -> Int { println("default executed"); return 1; }
locus Settings { params { n: Int = observe(); m: Int = 2; } }
main locus App {
    @effects(none: {stdio}) fn omitted() { let settings = Settings { }; }
    @effects(none: {stdio}) fn supplied() { let settings = Settings { n: 4, m: 5 }; }
    run() { }
}
fn main() { App { }; }
"#;

#[test]
fn parameter_default_execution_round_trips_without_authored_call_edges() {
    let (path, v) = artifact_with_outcome("initialization_execution", INITIALIZATION, false);
    topology_law::validate_law_account(&v, "initialization").unwrap();
    let rows = v["law"]["rows"].as_array().unwrap();
    for (subject, expected) in [("App::omitted", "violated"), ("App::supplied", "holds")] {
        assert_eq!(rows.iter().find(|r| r["name"] == subject).unwrap()["verdict"], expected);
    }
    let edges = v["relations"]["initializes"].as_array().unwrap();
    assert_eq!(edges.iter().filter(|e| e["from"] == "App::omitted").count(), 2);
    assert!(!edges.iter().any(|e| e["from"] == "App::supplied"));
    assert!(!v["relations"]["calls"].as_array().unwrap().iter().any(|e| e["from"] == "App::omitted"));
    let component = fleet_model::ComponentModel::decode(&v, "initialization").unwrap();
    for edge in edges {
        assert!(component.calls.contains(&(edge["from"].as_str().unwrap().to_string(), edge["to"].as_str().unwrap().to_string())));
    }
    assert!(component.calls.contains(&("Settings::params::n".to_string(), "observe".to_string())));
    let graph = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["topology", "graph"]).arg(&path).args(["--format", "mermaid"]).output().unwrap();
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    let rendered = String::from_utf8(graph.stdout).unwrap();
    assert!(rendered.contains("initializes n"), "{rendered}");

    let source = INITIALIZATION.replace("println(\"default executed\"); ", "");
    let (path, v) = artifact("initialization_pure", &source);
    topology_law::validate_law_account(&v, "pure initialization").unwrap();
    let check = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check").arg(path.parent().unwrap().join("app.hl"))
        .arg(format!("--check-topology={}", path.display())).output().unwrap();
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
}

#[test]
fn initializer_admission_rejects_missing_and_forged_omission_accounts() {
    let (_, v) = artifact_with_outcome("initialization_mutations", INITIALIZATION, false);
    for variant in 0..12 {
        let mut changed = v.clone();
        let edge = changed["relations"]["initializes"][0].clone();
        let construction = changed["relations"]["constructions"].as_array().unwrap().iter()
            .position(|r| r["function"] == edge["from"] && r["site"] == edge["site"]).unwrap();
        match variant {
            0 => { changed["relations"]["initializes"].as_array_mut().unwrap().remove(0); }
            1 => { changed["relations"]["constructions"].as_array_mut().unwrap().remove(construction); }
            2 => changed["relations"]["constructions"][construction]["supplied"] = json!([edge["field"]]),
            3 => changed["relations"]["initializes"][0]["field"] = json!("wrong"),
            4 => changed["relations"]["initializes"][0]["owner"]["name"] = json!("App"),
            5 => changed["relations"]["initializes"][0]["member"] = json!(99),
            6 => changed["relations"]["initializes"][0]["to"] = json!("observe"),
            7 => changed["relations"]["initializes"][0]["loop"] = json!(true),
            8 => changed["relations"]["initializes"].as_array_mut().unwrap().push(edge),
            9 => changed["relations"]["constructions"][construction]["supplied"] = json!(["n", "n"]),
            10 => { changed["relations"].as_object_mut().unwrap().remove("initializers"); }
            _ => changed["relations"]["initializes"][0]["member"] = json!(-1),
        }
        let error = topology_law::validate_law_account(&changed, "initialization mutation").unwrap_err();
        assert!(error.contains("initializ") || error.contains("constructor") || error.contains("parameter-default"), "variant {variant}: {error}");
    }
}

#[test]
fn initializer_metadata_and_tables_require_their_schema_and_body_kind() {
    let (_, v) = artifact_with_outcome("initialization_schema", INITIALIZATION, false);
    let body = v["law"]["fn_universe"].as_array().unwrap().iter()
        .position(|r| r["kind"] == "parameter_default").unwrap();
    let mut old = v.clone(); old["schema"] = json!("1.25");
    refused(old, "parameter_default bodies require their schema");
    let mut wrong = v.clone(); wrong["law"]["fn_universe"][body]["kind"] = json!("method");
    refused(wrong, "only default bodies may carry initializer metadata");
    let mut missing = v.clone(); missing["law"]["fn_universe"][body].as_object_mut().unwrap().remove("initializer_field");
    refused(missing, "complete declaration identity");

    // Even a document with no default bodies cannot smuggle the new relation
    // vocabulary into an older schema.
    let (_, mut no_defaults) = artifact("initialization_old_tables", "main locus App { } fn main() { App { }; }");
    no_defaults["schema"] = json!("1.25");
    no_defaults["relations"]["initializes"] = json!([]);
    refused(no_defaults, "initializer execution tables require topology schema 1.26");
}

#[test]
fn field_default_artifacts_keep_type_ownership_execution_and_rendering() {
    let source = INITIALIZATION.replace("locus Settings { params { n: Int = observe(); m: Int = 2; } }",
        "type Settings { n: Int = observe(); m: Int = 2; }");
    let (path, v) = artifact_with_outcome("field_initialization", &source, false);
    topology_law::validate_law_account(&v, "field defaults").unwrap();
    let field = v["law"]["fn_universe"].as_array().unwrap().iter().find(|row| row["display"] == "Settings::fields::n").unwrap();
    assert_eq!(field["kind"], "field_default");
    assert_eq!(field["owner_type"], "Settings");
    assert!(field.get("owner").is_none());
    assert!(v["law"]["types"].as_array().unwrap().iter().any(|row| row["name"] == "Settings" && row["display"] == "Settings"));
    let edges = v["relations"]["initializes"].as_array().unwrap();
    assert_eq!(edges.iter().filter(|row| row["from"] == "App::omitted").count(), 2);
    assert!(edges.iter().all(|row| row["owner"] == json!({"kind":"type", "name":"Settings"})));
    assert!(!v["sorts"]["loci"].as_array().unwrap().iter().any(|row| row == "Settings"));
    let component = fleet_model::ComponentModel::decode(&v, "field defaults").unwrap();
    assert!(component.calls.contains(&("App::omitted".to_string(), "Settings::fields::n".to_string())));
    assert!(component.calls.contains(&("Settings::fields::n".to_string(), "observe".to_string())));
    let graph = Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology", "graph"]).arg(path)
        .args(["--format", "mermaid"]).output().unwrap();
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    let rendered = String::from_utf8(graph.stdout).unwrap();
    for label in ["Settings [type]", "fields::n", "initializes n"] { assert!(rendered.contains(label), "missing {label}: {rendered}"); }
}

#[test]
fn field_default_admission_refuses_forged_type_and_locus_ownership() {
    let source = INITIALIZATION.replace("locus Settings { params { n: Int = observe(); m: Int = 2; } }",
        "type Settings { n: Int = observe(); m: Int = 2; }");
    let (_, v) = artifact_with_outcome("field_owner_mutations", &source, false);
    let body = v["law"]["fn_universe"].as_array().unwrap().iter().position(|row| row["display"] == "Settings::fields::n").unwrap();
    let definition = v["relations"]["initializers"].as_array().unwrap().iter().position(|row| row["function"] == "Settings::fields::n").unwrap();
    for variant in 0..13 {
        let mut changed = v.clone();
        match variant {
            0 => changed["law"]["fn_universe"][body]["owner_type"] = json!("App"),
            1 => changed["law"]["fn_universe"][body]["owner"] = json!("App"),
            2 => changed["law"]["fn_universe"][body]["kind"] = json!("parameter_default"),
            3 => changed["relations"]["initializers"][definition]["owner"]["kind"] = json!("locus"),
            4 => changed["relations"]["initializers"][definition]["kind"] = json!("parameter"),
            5 => { changed["law"]["types"] = json!([]); }
            6 => { changed["sorts"]["types"] = json!([]); }
            7 => { changed["sorts"]["types"] = json!({}); }
            8 => { changed["sorts"]["types"] = json!(["Settings", "Settings"]); }
            9 => { changed["relations"]["initializers"].as_array_mut().unwrap().remove(definition); }
            10 => { changed["schema"] = json!("1.26"); }
            11 => changed["law"]["fn_universe"][body]["name"] = json!("Settings::params::n"),
            _ => { let row = changed["law"]["types"][0].clone(); changed["law"]["types"].as_array_mut().unwrap().push(row); }
        }
        assert!(topology_law::validate_law_account(&changed, "field owner mutation").is_err(), "variant {variant}");
    }
}

#[test]
fn schema_126_locus_default_accounts_remain_readable() {
    let (_, mut v) = artifact_with_outcome("legacy_parameter_defaults", &INITIALIZATION.replace("run() { }", ""), false);
    assert!(v["law"]["types"].as_array().unwrap().is_empty());
    v["schema"] = json!("1.26");
    v["law"].as_object_mut().unwrap().remove("types");
    v["sorts"].as_object_mut().unwrap().remove("types");
    let definitions = v["relations"].as_object_mut().unwrap().remove("initializers").unwrap();
    v["relations"]["parameter_defaults"] = json!(definitions.as_array().unwrap().iter().map(|row|
        json!({"function":row["function"], "locus":row["owner"]["name"], "field":row["field"], "parameter":row["member"]})).collect::<Vec<_>>());
    for table in ["constructions", "initializes"] {
        for row in v["relations"][table].as_array_mut().unwrap() {
            let map = row.as_object_mut().unwrap();
            let owner = map.remove("owner").unwrap(); map.insert("locus".into(), owner["name"].clone());
            if let Some(member) = map.remove("member") { map.insert("parameter".into(), member); }
        }
    }
    topology_law::validate_law_account(&v, "schema 1.26 locus defaults").unwrap();
    let mut wrong = v; wrong["law"]["types"] = json!([]);
    refused(wrong, "type-owner tables require topology schema 1.27");
}

const ARGUMENT_DEFAULTS: &str = r#"
fn observe() -> Int { println("argument default executed"); return 1; }
fn calculate(n: Int = observe(), m: Int = 2) -> Int { return n; }
main locus App {
    @effects(none: {stdio}) fn omitted() { let n = calculate(); }
    @effects(none: {stdio}) fn supplied() { let n = calculate(4, 5); }
}
fn main() { App { }; }
"#;

#[test]
fn argument_default_artifacts_preserve_call_ownership_and_execution() {
    let (path,v)=artifact_with_outcome("argument_default",ARGUMENT_DEFAULTS,false);
    topology_law::validate_law_account(&v,"argument defaults").unwrap();
    let default=v["law"]["fn_universe"].as_array().unwrap().iter().find(|r| r["display"] == "calculate::args::n").unwrap();
    assert_eq!(default["kind"],"argument_default");assert_eq!(default["owner_function"],"calculate");
    assert!(default.get("owner").is_none() && default.get("owner_type").is_none());
    let calls=v["relations"]["argument_calls"].as_array().unwrap();
    assert_eq!(calls.iter().find(|c| c["from"] == "App::omitted").unwrap()["supplied"],0);
    assert_eq!(calls.iter().find(|c| c["from"] == "App::supplied").unwrap()["supplied"],2);
    let edges=v["relations"]["argument_evaluations"].as_array().unwrap();
    assert_eq!(edges.len(),2);assert!(edges.iter().all(|r| r["from"] == "App::omitted" && r["callee"] == "calculate"));
    assert!(!v["relations"]["calls"].as_array().unwrap().iter().any(|r| r["from"] == "App::omitted" && r["to"] == "calculate::args::n"));
    let component=fleet_model::ComponentModel::decode(&v,"arguments").unwrap();
    assert!(component.calls.contains(&("App::omitted".to_string(),"calculate::args::n".to_string())));
    let rendered=Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology","graph"]).arg(path).args(["--format","mermaid"]).output().unwrap();
    assert!(rendered.status.success(),"{}",String::from_utf8_lossy(&rendered.stderr));
    assert!(String::from_utf8_lossy(&rendered.stdout).contains("defaults n"));
}

#[test]
fn argument_admission_rejects_lost_edges_and_forged_omission_facts() {
    let (_,v)=artifact_with_outcome("argument_mutations",ARGUMENT_DEFAULTS,false);
    let body=v["law"]["fn_universe"].as_array().unwrap().iter().position(|r| r["display"] == "calculate::args::n").unwrap();
    let call=v["relations"]["argument_calls"].as_array().unwrap().iter().position(|r| r["from"] == "App::omitted").unwrap();
    for variant in 0..15 {
        let mut changed=v.clone();
        match variant {
            0 => { changed["relations"]["argument_evaluations"].as_array_mut().unwrap().remove(0); }
            1 => { changed["relations"]["argument_calls"].as_array_mut().unwrap().remove(call); }
            2 => changed["relations"]["argument_calls"][call]["supplied"]=json!(2),
            3 => changed["relations"]["argument_calls"][call]["supplied"]=json!(3),
            4 => changed["relations"]["argument_calls"][call]["supplied"]=json!(-1),
            5 => changed["relations"]["argument_evaluations"][0]["site"]=json!(99),
            6 => changed["relations"]["argument_evaluations"][0]["argument"]=json!(99),
            7 => changed["relations"]["argument_evaluations"][0]["parameter"]=json!("wrong"),
            8 => changed["relations"]["argument_evaluations"][0]["loop"]=json!(true),
            9 => changed["law"]["fn_universe"][body]["owner_function"]=json!("App::omitted"),
            10 => changed["law"]["fn_universe"][body]["owner"]=json!("App"),
            11 => changed["schema"]=json!("1.27"),
            12 => { let r=changed["relations"]["argument_evaluations"][0].clone();changed["relations"]["argument_evaluations"].as_array_mut().unwrap().push(r); }
            13 => changed["relations"]["argument_calls"][call]["callee"]=json!("observe"),
            _ => { changed["relations"].as_object_mut().unwrap().remove("initializers"); }
        }
        assert!(topology_law::validate_law_account(&changed,"argument mutation").is_err(),"variant {variant}");
    }
    let (_,mut old)=artifact("argument_old_tables","main locus App { } fn main() { App { }; }");
    old["schema"]=json!("1.27");old["relations"]["argument_calls"]=json!([]);
    refused(old,"argument evaluation tables require topology schema 1.28");
}

const CLOSURE_CHECKS: &str = r#"
fn observe() -> Int { println("closure operand executed"); return 0; }
locus Worker {
    params { n: Int = 0; }
    closure zeta { observe() + self.n ~~ self.n within 0; epoch explicit; }
    closure alpha { self.n ~~ self.n within 0; epoch explicit; }
    closure born { self.n ~~ self.n within 0; epoch birth; }
    fn trigger() { check_closures(); }
    fn idle() { }
}
main locus App {
    @effects(none: {stdio}) fn start() { let worker = Worker { }; worker.trigger(); }
}
fn main() { App { }; }
"#;

#[test]
fn closure_artifacts_preserve_exact_epoch_owner_and_checkpoint_accounts() {
    let (path,v)=artifact_with_outcome("closure_check",CLOSURE_CHECKS,false);
    topology_law::validate_law_account(&v,"closure execution").unwrap();
    let definitions=v["relations"]["closure_definitions"].as_array().unwrap();
    assert_eq!(definitions.len(),3);
    let zeta=definitions.iter().find(|r| r["name"]=="zeta").unwrap();
    assert_eq!(zeta["owner"],"Worker");assert_eq!(zeta["epoch"],"explicit");assert_eq!(zeta["closure"],0);
    let body=v["law"]["fn_universe"].as_array().unwrap().iter().find(|r| r["display"]=="Worker::closures::zeta").unwrap();
    assert_eq!(body["kind"],"closure_assertion");assert_eq!(body["owner"],"Worker");
    let checks=v["relations"]["closure_checks"].as_array().unwrap();assert_eq!(checks.len(),1);
    assert_eq!(checks[0]["function"],"Worker::trigger");
    let evaluations=v["relations"]["closure_evaluations"].as_array().unwrap();assert_eq!(evaluations.len(),2);
    assert_eq!(evaluations[0]["to"],"Worker::closures::zeta");assert_eq!(evaluations[1]["to"],"Worker::closures::alpha");
    assert!(!v["relations"]["calls"].as_array().unwrap().iter().any(|r| r["to"]=="Worker::closures::zeta"));
    let component=fleet_model::ComponentModel::decode(&v,"closures").unwrap();
    assert!(component.calls.contains(&("Worker::trigger".to_string(),"Worker::closures::zeta".to_string())));
    let rendered=Command::new(env!("CARGO_BIN_EXE_hale")).args(["topology","graph"]).arg(path).args(["--format","mermaid"]).output().unwrap();
    assert!(rendered.status.success(),"{}",String::from_utf8_lossy(&rendered.stderr));
    assert!(String::from_utf8_lossy(&rendered.stdout).contains("checks closure"));
}

#[test]
fn automatic_epoch_constructions_require_explicit_coverage_residue() {
    let source = r#"
locus Worker { params { n: Int = 0; } closure audit { self.n ~~ 0 within 0; epoch birth; } }
@effects(none: {stdio}) @budget(stack_bytes = 999999) fn construct() { Worker { }; }
main locus App { run() { construct(); } }
fn main() { App { }; }
"#;
    let (_, v) = artifact_with_outcome("automatic_epoch_coverage", source, false);
    topology_law::validate_law_account(&v, "automatic epoch").unwrap();
    assert_eq!(v["schema"], hale_types::topology::TOPOLOGY_SCHEMA);
    assert!(v["law"]["rows"].as_array().unwrap().iter().all(|row| row["verdict"] == "uncertified"));
    let mut missing = v.clone();
    missing["unknowns"].as_array_mut().unwrap().retain(|row| row["fn"] != "construct");
    refused(missing.clone(), "automatic closure construction requires scoped invocation-context coverage");
    missing["schema"] = json!("1.29");
    refused(missing, "guarded self lifecycle accounts require topology schema 1.34");
    let mut wrong_anchor = v.clone();
    wrong_anchor["unknowns"].as_array_mut().unwrap().iter_mut().find(|row| row["fn"] == "construct").unwrap()["fn"] = json!("App::run");
    refused(wrong_anchor, "automatic closure construction requires scoped invocation-context coverage");
    let mut downgrade = v;
    downgrade["schema"] = json!("1.29");
    refused(downgrade, "guarded self lifecycle accounts require topology schema 1.34");
    let (_, mut previous) = artifact_with_outcome("prior_epoch_without_authored_hook",
        &source.replace("main locus App { run() { construct(); } }", "main locus App { }"), false);
    previous["schema"] = json!("1.29");
    topology_law::validate_law_account(&previous, "prior explicit epoch account").unwrap();
}

#[test]
fn automatic_assertion_phase_contracts_round_trip_without_an_authored_hook() {
    let source = r#"
fn observe() -> Int { println("phase operand"); return 0; }
@phase_effects(birth: {})
locus Worker {
    params { n: Int = 0; }
    closure audit { observe() + self.n ~~ self.n within 0; epoch birth; }
}
fn main() { Worker { }; }
"#;
    let (_, v) = artifact_with_outcome("automatic_phase", source, false);
    topology_law::validate_law_account(&v, "automatic phase").unwrap();
    assert_eq!(v["law"]["rows"][0]["verdict"], "violated");
    assert_eq!(v["phases"]["Worker::closures::audit"], json!({"phase":"birth","kind":"hook"}));
    let quiet = source.replace("println(\"phase operand\");", "");
    let (_, quiet) = artifact_with_outcome("automatic_phase_unknown", &quiet, false);
    assert_eq!(quiet["law"]["rows"][0]["verdict"], "uncertified");
    topology_law::validate_law_account(&quiet, "conditional automatic phase").unwrap();
    for mutation in 0..5 {
        let mut changed = v.clone();
        match mutation {
            0 => { changed["phases"].as_object_mut().unwrap().remove("Worker::closures::audit"); }
            1 => changed["phases"]["Worker::closures::audit"]["phase"] = json!("dissolve"),
            2 => changed["phases"]["Worker::closures::audit"]["kind"] = json!("method"),
            3 => { changed["schema"] = json!("1.30"); changed["phases"].as_object_mut().unwrap().remove("Worker::closures::audit"); }
            _ => changed["relations"]["closure_definitions"][0]["epoch"] = json!("explicit"),
        }
        assert!(topology_law::validate_law_account(&changed, "phase mutation").is_err(), "mutation {mutation}");
    }
    let (_, mut authored) = artifact_with_outcome("authored_phase_account", "@phase_effects(birth: {}) locus Worker { birth() { println(1); } } fn main() { Worker { }; }", false);
    authored["phases"].as_object_mut().unwrap().remove("Worker::birth");
    refused(authored, "lifecycle definition identity, stage and locus ownership must agree");
}

#[test]
fn closure_admission_rejects_forged_definitions_lost_evaluations_and_downgrades() {
    let (_,v)=artifact_with_outcome("closure_mutations",CLOSURE_CHECKS,false);
    let body=v["law"]["fn_universe"].as_array().unwrap().iter().position(|r| r["display"]=="Worker::closures::zeta").unwrap();
    let definition=v["relations"]["closure_definitions"].as_array().unwrap().iter().position(|r| r["name"]=="zeta").unwrap();
    for variant in 0..22 {
        let mut changed=v.clone();
        match variant {
            0=> { changed["relations"]["closure_evaluations"].as_array_mut().unwrap().remove(0); }
            1=> { changed["relations"]["closure_checks"].as_array_mut().unwrap().clear(); }
            2=> { changed["relations"]["closure_definitions"].as_array_mut().unwrap().remove(definition); }
            3=>changed["relations"]["closure_definitions"][definition]["epoch"]=json!("birth"),
            4=>changed["relations"]["closure_checks"][0]["owner"]=json!("App"),
            5=>changed["relations"]["closure_evaluations"][0]["site"]=json!(99),
            6=>changed["relations"]["closure_evaluations"][0]["closure"]=json!(99),
            7=>changed["relations"]["closure_evaluations"][0]["to"]=json!("observe"),
            8=>changed["relations"]["closure_evaluations"][0]["loop"]=json!(true),
            9=> { changed["relations"]["closure_checks"][0]["unbounded"]=json!(true);for row in changed["relations"]["closure_evaluations"].as_array_mut().unwrap() { row["unbounded"]=json!(true); } }
            10=> { let row=changed["relations"]["closure_evaluations"][0].clone();changed["relations"]["closure_evaluations"].as_array_mut().unwrap().push(row); }
            11=> { let row=changed["relations"]["closure_checks"][0].clone();changed["relations"]["closure_checks"].as_array_mut().unwrap().push(row); }
            12=> { let row=changed["relations"]["closure_definitions"][0].clone();changed["relations"]["closure_definitions"].as_array_mut().unwrap().push(row); }
            13=>changed["schema"]=json!("1.28"),
            14=>changed["relations"]["closure_definitions"][definition]["owner"]=json!("App"),
            15=>changed["law"]["fn_universe"][body]["kind"]=json!("method"),
            16=>changed["relations"]["closure_definitions"][definition]["epoch"]=json!("inline"),
            17=>changed["relations"]["closure_definitions"][definition]["closure"]=json!(u32::MAX),
            18=>changed["relations"]["closure_checks"][0]["site"]=json!(-1),
            19=>changed["relations"]["closure_evaluations"][0]["unbounded"]=json!("false"),
            20=> { changed["relations"].as_object_mut().unwrap().remove("closure_definitions"); }
            _=>changed["relations"]["closure_checks"][0]["function"]=json!("observe"),
        }
        assert!(topology_law::validate_law_account(&changed,"closure mutation").is_err(),"variant {variant}");
    }
    let (_,mut legacy)=artifact("old_closure_table","main locus App { } fn main() { App { }; }");
    legacy["schema"]=json!("1.28");legacy["relations"]["closure_checks"]=json!([]);
    refused(legacy,"closure execution tables require topology schema 1.29");
}

#[test]
fn schema_128_argument_accounts_remain_readable() {
    let (_,mut v)=artifact_with_outcome("schema_128_arguments",ARGUMENT_DEFAULTS,false);
    v["schema"]=json!("1.28");
    topology_law::validate_law_account(&v,"schema 1.28 arguments").unwrap();
}

#[test]
fn birth_check_artifacts_preserve_the_clause_phase_and_failure_account() {
    let source = r#"
fn before() -> Bool { println("predicate"); return false; }
@phase_effects(birth: {}) locus Worker {
    closure rejected { epoch inline; }
    birth_check { before() } -> violate rejected;
    birth_check { false } -> violate rejected;
}
fn main() { Worker { }; }
"#;
    let (_, v) = artifact_with_outcome("birth_check_account", source, false);
    topology_law::validate_law_account(&v, "birth check").unwrap();
    assert_eq!(v["schema"], hale_types::topology::TOPOLOGY_SCHEMA);
    assert_eq!(v["law"]["rows"][0]["verdict"], "violated");
    assert_eq!(v["relations"]["birth_checks"].as_array().unwrap().len(), 2);
    assert_eq!(v["phases"]["Worker::birth_checks::0"], json!({"phase":"birth","kind":"hook"}));
    let (_, quiet) = artifact_with_outcome("birth_check_unknown", &source.replace("println(\"predicate\");", ""), false);
    topology_law::validate_law_account(&quiet, "conditional birth check").unwrap();
    assert_eq!(quiet["law"]["rows"][0]["verdict"], "uncertified");
    for mutation in 0..10 {
        let mut changed = v.clone();
        match mutation {
            0 => { changed["relations"].as_object_mut().unwrap().remove("birth_checks"); }
            1 => changed["relations"]["birth_checks"][0]["clause"] = json!(1),
            2 => changed["relations"]["birth_checks"][0]["closure"] = json!(""),
            3 => { changed["phases"].as_object_mut().unwrap().remove("Worker::birth_checks::0"); }
            4 => changed["phases"]["Worker::birth_checks::0"]["phase"] = json!("run"),
            5 => changed["phases"]["Worker::birth_checks::0"]["kind"] = json!("method"),
            6 => changed["unknowns"].as_array_mut().unwrap().retain(|r| r["fn"] != "Worker::birth_checks::0"),
            7 => changed["unknowns"].as_array_mut().unwrap().retain(|r| r["fn"] != "main"),
            8 => changed["schema"] = json!("1.31"),
            _ => changed["relations"]["birth_checks"].as_array_mut().unwrap().reverse(),
        }
        assert!(topology_law::validate_law_account(&changed, "birth check mutation").is_err(), "mutation {mutation}");
    }
}

#[test]
fn lifecycle_artifacts_keep_source_definitions_and_constructor_entries() {
    let source = r#"
fn observe() { println("hook ran"); }
locus Child { birth() { observe(); } }
@effects(none: {stdio}) fn construct() { Child { }; }
fn main() { construct(); }
"#;
    let (_, v)=artifact_with_outcome("lifecycle_entries",source,false);
    topology_law::validate_law_account(&v,"lifecycle").unwrap();
    assert_eq!(v["schema"],hale_types::topology::TOPOLOGY_SCHEMA);
    assert_eq!(v["law"]["rows"][0]["verdict"],"violated");
    assert_eq!(v["relations"]["lifecycle_definitions"][0],json!({"function":"Child::birth","owner":"Child","stage":"birth","empty":false}));
    assert_eq!(v["relations"]["lifecycle_evaluations"][0]["from"],"construct");
    assert_eq!(v["relations"]["lifecycle_evaluations"][0]["guard"],"construction");
    for variant in 0..11 {
        let mut changed=v.clone();
        match variant {
            0=> { changed["relations"].as_object_mut().unwrap().remove("lifecycle_definitions"); }
            1=> { changed["relations"].as_object_mut().unwrap().remove("lifecycle_evaluations"); }
            2=> changed["relations"]["lifecycle_evaluations"][0]["stage"]=json!("run"),
            3=> changed["relations"]["lifecycle_evaluations"][0]["site"]=json!(1),
            4=> changed["relations"]["lifecycle_definitions"][0]["empty"]=json!(true),
            5=> changed["unknowns"].as_array_mut().unwrap().retain(|u| u["fn"]!="construct"),
            6=> changed["schema"]=json!("1.32"),
            7=> changed["relations"]["lifecycle_evaluations"][0]["loop"]=json!(true),
            8=> changed["relations"]["lifecycle_evaluations"][0]["guard"]=json!("teardown"),
            9=> { changed["relations"]["lifecycle_evaluations"][0].as_object_mut().unwrap().remove("guard"); }
            _=> changed["schema"]=json!("1.33"),
        }
        assert!(topology_law::validate_law_account(&changed,"lifecycle mutation").is_err(),"variant {variant}");
    }
}

#[test]
fn later_self_stage_artifacts_bind_guards_without_discarding_context_gaps() {
    let base = r#"
fn observe() { println("later hook"); }
locus Child { run() { observe(); } }
@effects(none: {stdio}) fn construct() { Child { }; }
fn main() { construct(); }
"#;
    for (stage,guard) in [("run","run_admission"),("drain","teardown"),("dissolve","teardown")] {
        let source=base.replace("run()", &format!("{stage}()"));
        let (_,v)=artifact_with_outcome(&format!("later_stage_{stage}"),&source,false);
        topology_law::validate_law_account(&v,"later lifecycle stage").unwrap();
        assert_eq!(v["law"]["rows"][0]["verdict"],"violated");
        let edge=&v["relations"]["lifecycle_evaluations"][0];
        assert_eq!(edge["stage"],stage); assert_eq!(edge["guard"],guard);
        assert!(v["unknowns"].as_array().unwrap().iter().any(|r| r["fn"]=="construct"
            && r["reasons"].as_array().unwrap().iter().any(|r| r=="unmodeled_initialization")));
        for variant in 0..3 {
            let mut changed=v.clone();
            match variant {
                0=>changed["relations"]["lifecycle_evaluations"][0]["guard"]=json!("construction"),
                1=>changed["relations"]["lifecycle_evaluations"].as_array_mut().unwrap().clear(),
                _=>changed["schema"]=json!("1.33"),
            }
            assert!(topology_law::validate_law_account(&changed,"later stage mutation").is_err());
        }
    }
}
