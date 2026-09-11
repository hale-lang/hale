//! GH #583 M1 — the model catalog is source, and `hale dna models`
//! probes it.
//!
//! `init` writes `dna/org/models.hl` from what the machine has (keys in
//! the environment, servers on PATH) and the organization's main takes
//! every router from it; `hale dna models` builds the catalog beside a
//! one-line main and sends one small request to each backend, keyless
//! here: two scripted backends answer, a hosted one without its key is
//! not permitted, and nothing of the organization is started.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale_env(args: &[&str], cwd: &Path, env: &[(&str, &str)], unset: &[&str]) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd);
    for k in unset {
        c.env_remove(k);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

/// stdout alone: what the verb answers (the toolchain's build notices
/// go to stderr).
fn hale_stdout(args: &[&str], cwd: &Path, unset: &[&str]) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd);
    for k in unset {
        c.env_remove(k);
    }
    let out = c.output().expect("hale");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string())
}

/// A PATH with the toolchain's essentials and none of the discoverable
/// servers or harnesses, so the catalog written here is the same on
/// every machine.
fn bare_path() -> String {
    "/usr/bin:/bin".to_string()
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_dna_models_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const NO_KEYS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

/// The catalog the probe test installs: two scripted backends that
/// answer, one hosted backend with no key.
const SCRIPTED_CATALOG: &str = r#"// a catalog for the probe test
import "vendor/dna" as dna;

fn frontier() -> dna::FakeModel {
    return dna::FakeModel { name: "deep", model: "deep-1", answer: "ready\nand a second line", price_micros: 50 };
}
fn fast() -> dna::FakeModel {
    return dna::FakeModel { name: "quick", model: "quick-1", answer: "ready", price_micros: 5 };
}
fn desk() -> dna::OpenAiChat {
    return dna::OpenAiChat { name: "private", model: "gpt-4o", credential: dna::HostedCredential { env_var: "HALE_DNA_NO_SUCH_KEY_9c1e" } };
}
fn leader_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier() }; }
fn editor_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier() }; }
fn agent_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier() }; }
fn org_budget() -> dna::BudgetPolicy { return dna::BudgetPolicy { window: "none", allowance_micros: 1000 }; }
fn probe_catalog() -> String {
    return dna::probe("frontier", frontier()) + dna::probe("fast", fast()) + dna::probe("desk", desk());
}
"#;

#[test]
fn init_writes_the_catalog_from_what_the_machine_has_and_the_org_takes_its_routers_from_it() {
    // 1. nothing found: the catalog names OpenAI with a placeholder key
    let d = workdir("bare");
    let (ok, out) = hale_env(&["dna", "new", "bare"], &d, &[("PATH", &bare_path())], NO_KEYS);
    assert!(ok, "{out}");
    let app = d.join("bare");
    let catalog = std::fs::read_to_string(app.join("dna/org/models.hl")).expect("init writes the catalog");
    assert!(out.contains("created ") && out.contains("dna/org/models.hl"), "{out}");
    assert!(out.contains("models  found   no API key in the environment, no ollama on PATH"), "{out}");
    assert!(out.contains("frontier = gpt-4o · fast = gpt-4o-mini (OPENAI_API_KEY, not set: hosted backends are not permitted until it is) · desk = llama3 (ollama at 127.0.0.1:11434, not found)"), "{out}");
    assert!(out.contains("leader, editor, agent: deep = frontier, quick = fast, private = desk · budget 25.00 USD a day"), "{out}");
    for needle in [
        "fn frontier() -> dna::OpenAiChat",
        "model: \"gpt-4o\", endpoint: \"https://api.openai.com/v1/chat/completions\", credential: dna::HostedCredential { env_var: \"OPENAI_API_KEY\" }",
        "fn fast() -> dna::OpenAiChat",
        "fn desk() -> dna::LocalModel",
        "fn leader_models() -> dna::ModelRouter",
        "fn editor_models() -> dna::ModelRouter",
        "fn agent_models() -> dna::ModelRouter",
        "fn org_budget() -> dna::BudgetPolicy",
        "window: \"day\", allowance_micros: 25000000",
        "fn probe_catalog() -> String",
    ] {
        assert!(catalog.contains(needle), "missing {needle:?} in:\n{catalog}");
    }
    let main = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    for needle in ["models: leader_models()", "models: editor_models()", "models: agent_models()", "budget: dna::Budget { policy: org_budget() }"] {
        assert!(main.contains(needle), "the organization takes {needle:?} from the catalog:\n{main}");
    }
    assert!(!main.contains("dna::OpenAiChat") && !main.contains("HostedModel"), "no adapter is named inline in the main:\n{main}");
    // the organization still checks, with its law, and builds
    let (ok, out) = hale_env(&["check", "--matrix", "."], &app, &[], &[]);
    assert!(ok, "matrix: {out}");
    let (ok, out) = hale_env(&["build", "dna/org"], &app, &[], &[]);
    assert!(ok, "the organization builds from the catalog: {out}");

    // 2. an Anthropic key: the hosted backends speak to Anthropic's
    //    compatibility endpoint with the strongest models
    let d2 = workdir("anthropic");
    let (ok, out) = hale_env(&["dna", "new", "withkey"], &d2, &[("PATH", &bare_path()), ("ANTHROPIC_API_KEY", "sk-ant-test")], &["OPENAI_API_KEY"]);
    assert!(ok, "{out}");
    let catalog = std::fs::read_to_string(d2.join("withkey/dna/org/models.hl")).unwrap();
    assert!(out.contains("models  found   ANTHROPIC_API_KEY set, no ollama on PATH"), "{out}");
    assert!(out.contains("frontier = claude-opus-5 · fast = claude-haiku-4-5-20251001 (ANTHROPIC_API_KEY)"), "{out}");
    assert!(catalog.contains("model: \"claude-opus-5\", endpoint: \"https://api.anthropic.com/v1/chat/completions\", credential: dna::HostedCredential { env_var: \"ANTHROPIC_API_KEY\" }, input_micros_per_1k: 15000, output_micros_per_1k: 75000"), "{catalog}");
    assert!(catalog.contains("model: \"claude-haiku-4-5-20251001\""), "{catalog}");

    // 3. re-running keeps the catalog the project may have edited
    std::fs::write(d2.join("withkey/dna/org/models.hl"), "// mine\n").unwrap();
    let (ok, out) = hale_env(&["dna", "init", "."], &d2.join("withkey"), &[("PATH", &bare_path())], NO_KEYS);
    assert!(ok, "{out}");
    assert_eq!(std::fs::read_to_string(d2.join("withkey/dna/org/models.hl")).unwrap(), "// mine\n", "kept on re-run");
    assert!(!out.contains("models  found"), "no discovery report when the catalog is kept:\n{out}");
}

#[test]
fn upgrade_gives_an_older_organization_a_catalog_and_says_what_to_point_at_it() {
    let d = workdir("upgrade");
    let (ok, out) = hale_env(&["dna", "new", "older"], &d, &[("PATH", &bare_path())], NO_KEYS);
    assert!(ok, "{out}");
    let app = d.join("older");
    // an organization from before the catalog: routers inline, no models.hl
    std::fs::remove_file(app.join("dna/org/models.hl")).unwrap();
    let main = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    std::fs::write(
        app.join("dna/org/main.hl"),
        main.replace("models: leader_models()", "models: dna::ModelRouter { quick: dna::HostedModel { name: \"quick\" } }"),
    )
    .unwrap();
    let (ok, out) = hale_env(&["dna", "upgrade"], &app, &[("PATH", &bare_path())], NO_KEYS);
    assert!(ok, "{out}");
    assert!(app.join("dna/org/models.hl").is_file(), "upgrade writes the catalog: {out}");
    assert!(out.contains("created ") && out.contains("models  found   no API key"), "{out}");
    assert!(out.contains("note    dna/org/main.hl wires its routers inline (dna::HostedModel is now dna::OpenAiChat); point each position at the catalog: `models: leader_models()`"), "{out}");
    // a second upgrade has nothing to add
    let (ok, out) = hale_env(&["dna", "upgrade"], &app, &[("PATH", &bare_path())], NO_KEYS);
    assert!(ok && !out.contains("models  ") && !out.contains("note    "), "{out}");
}

#[test]
fn models_probes_every_backend_of_the_catalog_without_starting_the_organization() {
    let d = workdir("probe");
    let (ok, out) = hale_env(&["dna", "new", "probed"], &d, &[("PATH", &bare_path())], NO_KEYS);
    assert!(ok, "{out}");
    let app = d.join("probed");
    std::fs::write(app.join("dna/org/models.hl"), SCRIPTED_CATALOG).unwrap();
    let (ok, out) = hale_stdout(&["dna", "models"], &app, &["HALE_DNA_NO_SUCH_KEY_9c1e"]);
    assert!(ok, "{out}");
    assert!(out.starts_with("catalog dna/org/models.hl\nbackend     slot      model                         answer\n"), "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 5, "one line per backend:\n{out}");
    assert!(lines[2].starts_with("frontier    deep      deep-1                        ok  "), "{out}");
    assert!(lines[2].contains("  50 micro-dollars  \"ready\""), "the answer's first line, and what it cost:\n{out}");
    assert!(lines[3].starts_with("fast        quick     quick-1                       ok  ") && lines[3].contains("  5 micro-dollars  \"ready\""), "{out}");
    assert_eq!(lines[4], "desk        private   gpt-4o                        not permitted (no credential present)", "{out}");
    // the probe is scratch; the organization was not started, the
    // membrane not bound, the record not written
    assert!(app.join(".hale/dna/probe/probe").is_file(), "built in scratch");
    assert!(!app.join(".hale/dna/org.pid").exists(), "no organization started");
    assert!(!app.join(".hale/dna/hale-dna.review.verdict.sock").exists(), "no membrane bound");
    let (ok, hist) = hale_env(&["dna", "history"], &app, &[], &[]);
    assert!(ok && !hist.contains("model.called"), "a probe is nobody's Attempt: nothing journaled\n{hist}");

    // a catalog that does not build is named, not swallowed
    std::fs::write(app.join("dna/org/models.hl"), "import \"vendor/dna\" as dna;\nfn probe_catalog() -> String { return dna::probe(\"x\", nothing()); }\n").unwrap();
    let (ok, out) = hale_env(&["dna", "models"], &app, &[], &[]);
    assert!(!ok && out.contains("the catalog does not build"), "{out}");
}
