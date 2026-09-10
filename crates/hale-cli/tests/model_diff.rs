//! GH #527 B4 — `hale model diff` over two topology artifacts.
//!
//! What these hold the tool to:
//!   1. Artifact client only, both inputs admitted through the same
//!      gates as the renderer: an edited artifact is refused.
//!   2. Deterministic matching that names what it compared: a rename
//!      with one unique signature match is a rename; two candidates
//!      are reported as ambiguous, never guessed.
//!   3. The classification is honest: a comment-only edit is
//!      source-only (shape_hash unchanged); a topology edit is
//!      model-shape; the same artifact twice is identical.
//!   4. Contract, effect and law deltas are per-locus / per-fn /
//!      per-claim set differences with both sides' spans.
//!   5. Byte-determinism: the same pair diffs to the same bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale_modeldiff_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Write `src` as `<dir>/<name>/main.hl`, dump its artifact, return the artifact path.
fn dump(dir: &Path, name: &str, src: &str) -> PathBuf {
    let seed = dir.join(name);
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), src).unwrap();
    let artifact = dir.join(format!("{name}.topology"));
    let out = hale()
        .arg("check")
        .arg(&seed)
        .arg(format!("--dump-topology={}", artifact.display()))
        .output()
        .expect("hale check");
    assert!(
        artifact.is_file(),
        "no artifact for {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    artifact
}

fn diff_json(a: &Path, b: &Path) -> serde_json::Value {
    let out = hale().args(["model", "diff"]).arg(a).arg(b).output().expect("hale model diff");
    assert!(out.status.success(), "diff failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("diff is JSON")
}

fn diff_text(a: &Path, b: &Path) -> String {
    let out = hale().args(["model", "diff", "--text"]).arg(a).arg(b).output().expect("hale model diff --text");
    assert!(out.status.success(), "diff failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn rows<'a>(v: &'a serde_json::Value, section: &str) -> &'a [serde_json::Value] {
    v[section].as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

fn find<'a>(rs: &'a [serde_json::Value], change: &str, name: &str) -> Option<&'a serde_json::Value> {
    rs.iter().find(|r| r["change"] == change && r["name"] == name)
}

const BASE: &str = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; }
    bus { publish Evt; }
    claims {
        consumed: require subscribes(some workers, topic Evt);
    }
    run() { Evt <- T { n: 1 }; }
}
fn main() { App { }; }
"#;

#[test]
fn identical_and_source_only_and_model_shape_are_told_apart() {
    let dir = workdir("class");
    let a = dump(&dir, "a", BASE);
    let same = diff_json(&a, &a);
    assert_eq!(same["schema"], "1.0");
    assert_eq!(same["classification"], "identical");
    assert_eq!(same["a"]["artifact_digest"], same["b"]["artifact_digest"]);

    // A leading comment moves every span and the file digest; the
    // model half is untouched.
    let commented = format!("// a comment that moves everything\n{BASE}");
    let b = dump(&dir, "b", &commented);
    let d = diff_json(&a, &b);
    assert_eq!(d["classification"], "source-only");
    assert_eq!(d["a"]["shape_hash"], d["b"]["shape_hash"]);
    assert_ne!(d["a"]["artifact_digest"], d["b"]["artifact_digest"]);
    assert!(rows(&d, "declarations").iter().all(|r| r["change"] == "persisted"), "{d}");
    assert!(rows(&d, "contracts").is_empty());
    let text = diff_text(&a, &b);
    assert!(text.contains("classification: source-only"), "{text}");
    assert!(text.contains("no semantic differences"), "{text}");

    // Adding a topic is a shape change.
    let extended = BASE.replace(
        "topic Evt { payload: T; subject: \"evt\"; }",
        "topic Evt { payload: T; subject: \"evt\"; }\ntopic Late { payload: T; subject: \"late\"; }",
    );
    let c = dump(&dir, "c", &extended);
    let d = diff_json(&a, &c);
    assert_eq!(d["classification"], "model-shape");
    let added = find(rows(&d, "declarations"), "added", "Late").expect("Late added");
    assert_eq!(added["kind"], "topic");
    assert_eq!(added["b"]["unit"], "main.hl", "span is relative to the source root: {added}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_unique_signature_match_is_a_rename_with_contracts_carried_over() {
    let dir = workdir("rename");
    let a = dump(&dir, "a", BASE);
    let renamed = BASE.replace("Worker", "Crew").replace("workers", "crews");
    let b = dump(&dir, "b", &renamed);
    let d = diff_json(&a, &b);
    let decls = rows(&d, "declarations");
    let r = find(decls, "renamed", "Crew").expect("Worker -> Crew is a rename");
    assert_eq!(r["kind"], "locus");
    assert_eq!(r["from"], "Worker");
    assert!(r["a"]["span"].is_array() && r["b"]["span"].is_array(), "both sides carry spans: {r}");
    // The locus rename is read through: the group whose only member
    // was renamed is itself a rename, the method pairs under its new
    // owner, and App's `w: Worker` -> `w: Crew` is not a contract
    // delta.
    let g = find(decls, "renamed", "crews").expect("workers -> crews is a rename");
    assert_eq!(g["from"], "workers");
    let m = find(decls, "renamed", "Crew::on_e").expect("Worker::on_e -> Crew::on_e pairs");
    assert_eq!(m["from"], "Worker::on_e");
    assert!(rows(&d, "contracts").is_empty(), "a pure rename has no contract deltas: {}", d["contracts"]);
    assert!(rows(&d["effects"], "classes").is_empty(), "{}", d["effects"]);
    assert!(rows(&d["law"], "claims").is_empty(), "{}", d["law"]);
    assert_eq!(d["renames"]["Worker"], "Crew");
    assert_eq!(d["summary"]["renamed"], 3);
    let text = diff_text(&a, &b);
    assert!(text.contains("~ locus Worker -> Crew"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_candidates_are_reported_ambiguous_never_guessed() {
    let dir = workdir("ambig");
    let a = dump(&dir, "a", BASE);
    // Worker disappears; two new loci share its exact signature.
    let two = BASE
        .replace(
            "locus Worker {\n    params { seen: Int = 0; }\n    bus { subscribe Evt as on_e; }\n    fn on_e(t: T) { self.seen = self.seen + 1; }\n}",
            "locus Left {\n    params { seen: Int = 0; }\n    bus { subscribe Evt as on_e; }\n    fn on_e(t: T) { self.seen = self.seen + 1; }\n}\nlocus Right {\n    params { seen: Int = 0; }\n    bus { subscribe Evt as on_e; }\n    fn on_e(t: T) { self.seen = self.seen + 1; }\n}",
        )
        .replace("group workers = { Worker };", "group workers = { Left, Right };")
        .replace("params { w: Worker = Worker { }; }", "params { l: Left = Left { }; r: Right = Right { }; }");
    let b = dump(&dir, "b", &two);
    let d = diff_json(&a, &b);
    let decls = rows(&d, "declarations");
    let amb = find(decls, "ambiguous", "Worker").expect("Worker's rename is ambiguous");
    let cands: Vec<String> = amb["candidates"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    assert_eq!(cands, vec!["Left", "Right"]);
    assert!(find(decls, "renamed", "Left").is_none() && find(decls, "renamed", "Right").is_none(), "no guessed rename: {d}");
    assert!(find(decls, "added", "Left").is_some() && find(decls, "added", "Right").is_some(), "{d}");
    assert_eq!(d["summary"]["ambiguous"], 1);
    let text = diff_text(&a, &b);
    assert!(text.contains("? locus Worker  rename ambiguous: candidates Left, Right"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_split_is_an_exact_method_partition() {
    let dir = workdir("split");
    let before = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
topic Cmd { payload: T; subject: "cmd"; }
locus Both {
    bus { subscribe Evt as on_e; subscribe Cmd as on_c; }
    fn on_e(t: T) { println("e"); }
    fn on_c(t: T) { println("c"); }
}
main locus App {
    params { b: Both = Both { }; }
    bus { publish Evt; publish Cmd; }
    run() { Evt <- T { n: 1 }; Cmd <- T { n: 2 }; }
}
fn main() { App { }; }
"#;
    let after = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
topic Cmd { payload: T; subject: "cmd"; }
locus Events {
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { println("e"); }
}
locus Commands {
    bus { subscribe Cmd as on_c; }
    fn on_c(t: T) { println("c"); }
}
main locus App {
    params { e: Events = Events { }; c: Commands = Commands { }; }
    bus { publish Evt; publish Cmd; }
    run() { Evt <- T { n: 1 }; Cmd <- T { n: 2 }; }
}
fn main() { App { }; }
"#;
    let a = dump(&dir, "a", before);
    let b = dump(&dir, "b", after);
    let d = diff_json(&a, &b);
    let decls = rows(&d, "declarations");
    let s = find(decls, "split", "Both").expect("Both split");
    let into: Vec<&str> = s["into"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(into, vec!["Commands", "Events"]);
    assert!(find(decls, "added", "Events").is_none(), "split members are not also 'added': {d}");
    // and the mirror is a join
    let d2 = diff_json(&b, &a);
    let j = find(rows(&d2, "declarations"), "joined", "Both").expect("Both joined");
    let from: Vec<&str> = j["from"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(from, vec!["Commands", "Events"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn contract_effect_and_law_deltas_are_per_entity_set_differences() {
    let dir = workdir("deltas");
    let a = dump(&dir, "a", BASE);
    let changed = BASE
        // App gains a param, a second publish, and Worker's handler
        // gains a publish effect.
        .replace("topic Evt { payload: T; subject: \"evt\"; }", "topic Evt { payload: T; subject: \"evt\"; }\ntopic Out { payload: T; subject: \"out\"; }")
        .replace(
            "    bus { subscribe Evt as on_e; }\n    fn on_e(t: T) { self.seen = self.seen + 1; }",
            "    bus { subscribe Evt as on_e; publish Out; }\n    fn on_e(t: T) { self.seen = self.seen + 1; Out <- T { n: 2 }; }",
        )
        .replace("params { w: Worker = Worker { }; }", "params { w: Worker = Worker { }; limit: Int = 3; }")
        .replace(
            "consumed: require subscribes(some workers, topic Evt);",
            "consumed: require subscribes(some workers, topic Evt);\n        quiet: count publishers(topic Out) == 0;",
        );
    let b = dump(&dir, "b", &changed);
    let d = diff_json(&a, &b);
    let contracts = rows(&d, "contracts");
    // a wider payload with the same topology is a CONTRACT change, never "source-only"
    let widened = BASE.replace("type T { n: Int = 0; }", "type T { n: Int = 0; note: String = \"\"; }");
    let w = dump(&dir, "w", &widened);
    let dw = diff_json(&a, &w);
    assert_eq!(dw["classification"], "contract", "{dw}");
    assert!(rows(&dw, "contracts").iter().any(|r| r["topic"] == "Evt" && r["facet"] == "shape"), "{dw}");
    let app_params = contracts.iter().find(|r| r["locus"] == "App" && r["facet"] == "params").expect("App params delta");
    assert_eq!(app_params["added"], serde_json::json!(["limit: Int"]));
    assert_eq!(app_params["removed"], serde_json::json!([]));
    assert_eq!(app_params["a"]["unit"], "main.hl");
    let worker_pub = contracts.iter().find(|r| r["locus"] == "Worker" && r["facet"] == "publishes").expect("Worker publishes delta");
    assert_eq!(worker_pub["added"], serde_json::json!(["Out"]));
    let eff = rows(&d["effects"], "classes").iter().find(|r| r["fn"] == "Worker::on_e").expect("Worker::on_e effect delta");
    assert!(eff["gained"].as_array().unwrap().iter().any(|c| c == "publish"), "{eff}");
    let law = rows(&d["law"], "claims");
    let quiet = law.iter().find(|r| r["claim"] == "quiet").expect("quiet claim row");
    assert_eq!(quiet["change"], "added");
    assert_ne!(quiet["result"], "holds", "{quiet}");
    assert_eq!(d["law"]["verdict"]["a"], "clean");
    assert_ne!(d["law"]["verdict"]["b"], "clean");
    let text = diff_text(&a, &b);
    assert!(text.contains("! locus App params: +limit: Int"), "{text}");
    assert!(text.contains("! fn Worker::on_e gains"), "{text}");
    assert!(text.contains("+ claim quiet"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An added locus whose handler reaches a declared effect class is an
/// effect delta even though no paired fn changed: the row carries
/// `change: added` with the classes as `gained` (and a removed fn its
/// classes as `dropped`), so a magnitude read from the diff sees the
/// program widen (GH #529 D3).
#[test]
fn a_one_sided_fn_with_effects_is_an_effect_row() {
    let dir = workdir("sided");
    let a = dump(&dir, "a", BASE);
    let with_mailer = BASE
        .replace("type T { n: Int = 0; }", "effect mail;\ntype T { n: Int = 0; }")
        .replace(
            "group workers = { Worker };",
            "locus Mailer {\n    bus { subscribe Evt as on_e; }\n    @effects(is: { mail })\n    fn on_e(t: T) { }\n}\ngroup workers = { Worker };",
        )
        .replace("params { w: Worker = Worker { }; }", "params { w: Worker = Worker { }; m: Mailer = Mailer { }; }");
    let b = dump(&dir, "b", &with_mailer);
    let d = diff_json(&a, &b);
    let classes = rows(&d["effects"], "classes");
    let added = classes.iter().find(|r| r["fn"] == "Mailer::on_e").expect("Mailer::on_e is an effect row");
    assert_eq!(added["change"], "added", "{added}");
    assert!(added["gained"].as_array().unwrap().iter().any(|c| c == "mail"), "{added}");
    assert_eq!(added["dropped"], serde_json::json!([]));
    assert!(!classes.iter().any(|r| r["fn"] == "Worker::on_e"), "an unchanged paired fn has no row: {}", d["effects"]);
    assert_eq!(d["summary"]["effect_deltas"], 1, "{}", d["summary"]);
    let text = diff_text(&a, &b);
    assert!(text.contains("+ fn Mailer::on_e reaches mail"), "{text}");
    // and the other direction: the same fn leaving is a dropped row
    let back = diff_json(&b, &a);
    let removed = rows(&back["effects"], "classes").iter().find(|r| r["fn"] == "Mailer::on_e").cloned().expect("removed row");
    assert_eq!(removed["change"], "removed", "{removed}");
    assert!(removed["dropped"].as_array().unwrap().iter().any(|c| c == "mail"), "{removed}");
    assert!(diff_text(&b, &a).contains("- fn Mailer::on_e reached mail"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edited_artifact_is_refused_and_the_diff_is_byte_deterministic() {
    let dir = workdir("refuse");
    let a = dump(&dir, "a", BASE);
    let b = dump(&dir, "b", &format!("// moved\n{BASE}"));
    let x = diff_json(&a, &b);
    let y = diff_json(&a, &b);
    assert_eq!(x, y);
    let t1 = diff_text(&a, &b);
    let t2 = diff_text(&a, &b);
    assert_eq!(t1, t2);
    // Edit a claim result in place; the digest no longer verifies.
    let raw = std::fs::read_to_string(&b).unwrap();
    let edited = raw.replacen("\"result\": \"holds\"", "\"result\": \"violated\"", 1);
    assert_ne!(raw, edited, "fixture must carry a holding claim");
    let e = dir.join("edited.topology");
    std::fs::write(&e, edited).unwrap();
    let out = hale().args(["model", "diff"]).arg(&a).arg(&e).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("artifact_digest does not match"), "{err}");
    // Usage errors are exit 2, never a silent success.
    let out = hale().args(["model", "diff"]).arg(&a).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = hale().args(["model", "diff", "--bogus"]).arg(&a).arg(&b).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}
