//! What a seed provides the habitat's other organisms, declared in its
//! manifest:
//!
//! ```text
//! [dna.provides.models]
//! service     = "api"                  # the compose service that serves it (required)
//! path        = "/v1/chat/completions" # where a chat is asked (this default)
//! models_path = "/v1/models"           # where the models it serves are listed (this default)
//! key         = "tenant-key"           # a drawn family it declares: a member per tenant (required)
//!
//! [dna.secrets.tenant-key]
//! kind    = "drawn"
//! family  = true
//! readers = ["api"]
//! ```
//!
//! `models` is the one capability: an OpenAI-compatible endpoint the habitat
//! points every tenant's model map at, each tenant asking with a member of
//! the `key` family the habitat draws for it. This is the one reader of the
//! declaration: the DNA's own code asks it through `hale dna
//! declared-provides [dir]` (`lines`), and the habitat's graft reads it from
//! each organism's manifest (`dna::habitat::models`).

use std::collections::BTreeMap;

use super::secrets;

/// The capabilities a seed may provide.
const CAPABILITIES: &[&str] = &["models"];

/// A `[dna.provides.models]`, checked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Models {
    /// the compose service whose published port serves it
    pub service: String,
    /// where a chat is asked, under the service's address
    pub path: String,
    /// where the models it serves are listed
    pub models_path: String,
    /// the drawn family its tenants' keys are members of
    pub key: String,
    /// the object on each models-list entry holding the provider's own
    /// fields (`available`, `capabilities.tools`), when it keeps them
    /// there rather than beside `id` (voice: `voice`)
    pub extension: Option<String>,
    /// what a person does when the provider serves a tenant nothing yet,
    /// in the provider's words, `{organism}` standing for the tenant and
    /// `{habitat}` for the habitat's name
    pub grant: Option<String>,
}

pub(crate) const DEFAULT_PATH: &str = "/v1/chat/completions";
pub(crate) const DEFAULT_MODELS_PATH: &str = "/v1/models";

/// A compose service's name, as compose takes one: letters, digits, `-`,
/// `_` and `.`, a letter or a digit first.
fn service_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && s.starts_with(|c: char| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// A path under the service's address: `/` first, then what a URL's path
/// holds (no space, query, fragment or `..` segment).
fn path_ok(p: &str) -> bool {
    p.starts_with('/') && p.len() <= 256 && !p.split('/').any(|seg| seg == "..") && p.chars().all(|c| c.is_ascii_alphanumeric() || "/-_.~".contains(c))
}

/// What a manifest provides (`[dna.provides.<capability>]`), checked: the
/// `models` capability, or none.
pub(crate) fn declared(manifest: &str) -> Result<Option<Models>, String> {
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| format!("hale.toml: {e}"))?;
    let Some(provides) = doc.get("dna").and_then(|d| d.get("provides")) else { return Ok(None) };
    let provides = provides.as_table().ok_or("[dna.provides] is a table of capabilities, one `[dna.provides.<capability>]` each")?;
    let mut out = None;
    for (cap, t) in provides {
        if !CAPABILITIES.contains(&cap.as_str()) {
            return Err(format!("[dna.provides.{cap}]: a seed provides {} alone", CAPABILITIES.join(", ")));
        }
        let t = t.as_table().ok_or_else(|| format!("[dna.provides.{cap}] is a table"))?;
        if let Some(k) = t.keys().find(|k| !["service", "path", "models_path", "key", "extension", "grant"].contains(&k.as_str())) {
            return Err(format!("[dna.provides.{cap}] has no `{k}` (service, path, models_path, key, extension, grant)"));
        }
        let word = |k: &str| -> Result<Option<String>, String> { t.get(k).map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("[dna.provides.{cap}] {k} is a string"))).transpose() };
        let service = word("service")?.ok_or_else(|| format!("[dna.provides.{cap}] names its service: the compose service that serves it"))?;
        if !service_ok(&service) {
            return Err(format!("[dna.provides.{cap}] service `{service}`: a compose service's name (letters, digits, `-`, `_`, `.`)"));
        }
        let path = word("path")?.unwrap_or_else(|| DEFAULT_PATH.to_string());
        let models_path = word("models_path")?.unwrap_or_else(|| DEFAULT_MODELS_PATH.to_string());
        for (k, p) in [("path", &path), ("models_path", &models_path)] {
            if !path_ok(p) {
                return Err(format!("[dna.provides.{cap}] {k} `{p}`: a path under the service's address, `/` first, with no space, query or `..`"));
            }
        }
        let key = word("key")?.ok_or_else(|| format!("[dna.provides.{cap}] names its key: a drawn family it declares (`[dna.secrets.<name>] kind = \"drawn\", family = true`), a member per tenant"))?;
        if doc.get("dna").and_then(|d| d.get("secrets")).and_then(|t| t.get(&key)).is_none() {
            return Err(format!("[dna.provides.{cap}] key `{key}`: the seed declares no secret `{key}` ([dna.secrets.{key}], a drawn family)"));
        }
        let secrets = secrets::declared(manifest)?;
        match secrets.get(&key) {
            Some(d) if d.kind == secrets::Kind::Drawn && d.family => {}
            Some(_) => return Err(format!("[dna.provides.{cap}] key `{key}`: a drawn family (`kind = \"drawn\"`, `family = true` in [dna.secrets.{key}]), whose members the habitat draws, one per tenant")),
            None => return Err(format!("[dna.provides.{cap}] key `{key}`: the seed declares no secret `{key}` ([dna.secrets.{key}], a drawn family)")),
        }
        let extension = word("extension")?;
        if let Some(e) = &extension {
            if e.is_empty() || e.len() > 32 || !e.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                return Err(format!("[dna.provides.{cap}] extension `{e}`: the name of an object on each models-list entry (lowercase letters, digits, `_`, `-`)"));
            }
        }
        let grant = word("grant")?;
        if let Some(g) = &grant {
            if g.trim().is_empty() || g.len() > 240 || g.contains('\n') || g.contains('\t') {
                return Err(format!("[dna.provides.{cap}] grant: one line of at most 240 characters, what a person does when a tenant is served nothing yet (`{{organism}}` stands for the tenant, `{{habitat}}` for the habitat's name)"));
            }
        }
        out = Some(Models { service, path, models_path, key, extension, grant });
    }
    Ok(out)
}

/// The declaration as `hale dna declared-provides` prints it for the DNA's
/// code, a capability a line: `models\t<service>\t<path>\t<models_path>\t<key>\t<extension>\t<grant>`
/// (the last two empty when not declared).
pub(crate) fn lines(m: &Option<Models>) -> String {
    match m {
        Some(m) => format!("models\t{}\t{}\t{}\t{}\t{}\t{}\n", m.service, m.path, m.models_path, m.key, m.extension.as_deref().unwrap_or(""), m.grant.as_deref().unwrap_or("")),
        None => String::new(),
    }
}

/// `hale dna declared-provides [dir]`: what `<dir>/hale.toml` provides
/// (`lines`); nothing when it has no manifest or provides nothing, exit 1
/// and why when the declaration is refused.
pub(crate) fn declared_cmd(args: &[String]) -> std::process::ExitCode {
    let dir = args.first().map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = dir.join("hale.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return std::process::ExitCode::SUCCESS };
    match declared(&text) {
        Ok(m) => {
            print!("{}", lines(&m));
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("{}: {why}", path.display());
            std::process::ExitCode::from(1)
        }
    }
}

/// A tenant's member label: its organism's name, when the label rule
/// takes it (`secrets::label_ok`), else why it cannot be one.
pub(crate) fn tenant_label(organism: &str) -> Result<String, String> {
    if secrets::label_ok(organism) {
        Ok(organism.to_string())
    } else {
        Err(format!("the organism's name `{organism}` cannot label its key: a label is 1 to 63 characters, a lowercase letter or a digit, then lowercase letters, digits and `-`"))
    }
}

/// One model a provider serves, as its models list says it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Served {
    pub id: String,
    /// whether it returns tool calls (`capabilities.tools`; false unsaid)
    pub tools: bool,
}

/// The models an OpenAI-shaped models list serves the key that asked, in
/// its order: `{"data": [{"id": "...", "available": true, "capabilities":
/// {"tools": true}}, …]}`, or with the provider's own fields in the object
/// `extension` names on each entry (`{"id": "...", "voice": {"available":
/// …, "capabilities": …}}`). An entry that says it is not available is
/// one the key cannot be served by and is left out; one that does not say
/// is served. A model whose `capabilities.tools` is not said returns no
/// tool calls.
pub(crate) fn served(body: &str, extension: Option<&str>) -> Result<Vec<Served>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("its models list is not JSON: {e}"))?;
    let data = v["data"].as_array().ok_or("its models list has no `data` array")?;
    Ok(data
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str().filter(|s| !s.is_empty())?;
            let own = match extension {
                Some(e) => &m[e],
                None => m,
            };
            if own["available"].as_bool() == Some(false) {
                return None;
            }
            Some(Served { id: id.to_string(), tools: own["capabilities"]["tools"].as_bool().unwrap_or(false) })
        })
        .collect())
}

/// What the habitat called `habitat` says when the provider `provider`
/// serves the tenant `organism` nothing yet: the provider's own `grant`
/// sentence when it declares one (`{organism}` the tenant, `{habitat}` the
/// habitat's name, as its services are handed it), and how to map them
/// after.
pub(crate) fn serve_none(provider: &str, organism: &str, habitat: &str, grant: Option<&str>, why: Option<&str>) -> String {
    let why = why.map(|w| format!(" ({w})")).unwrap_or_default();
    match grant {
        Some(g) => format!("the habitat's models ({provider}) serve none for {organism} yet{why}: {}, then `hale habitat models {organism}`", g.replace("{organism}", organism).replace("{habitat}", habitat)),
        None => format!("the habitat's models ({provider}) serve none for {organism} yet{why}: `hale habitat models {organism}` maps them once one is served"),
    }
}

/// Who the habitat's own model rows are written by: a row in any other
/// name is a person's (or another act's) and the habitat never writes
/// over it.
pub(crate) const SEEDED_BY: &str = "hale habitat";

/// The rows of a tenant's model map the habitat writes from what its
/// models provider serves: `model.rule default` at `standard`, permitting
/// every size; every `think.<size>` the first served model; every
/// `tools.<size>` and `do.<size>` the first served model that returns tool
/// calls (none when none does); each asked at `endpoint` with the key in
/// the vault slot `key_vault`. `latest` is the tenant's latest row of each
/// `(kind, entity)` (its body): one in another name than the habitat's is
/// kept, and one the habitat wrote that already says the same is not
/// written again. Each row is `(kind, entity, body)`.
pub(crate) fn seed_rows(served: &[Served], latest: &BTreeMap<(String, String), serde_json::Value>, endpoint: &str, key_vault: &str) -> Vec<(String, String, serde_json::Value)> {
    let mut out = Vec::new();
    let writable = |kind: &str, entity: &str, body: &serde_json::Value| -> bool {
        match latest.get(&(kind.to_string(), entity.to_string())) {
            None => true,
            Some(had) if had["by"].as_str() != Some(SEEDED_BY) => false,
            Some(had) => {
                // the habitat's own: written again only when it says otherwise
                let fields = ["model", "size", "permit", "tool_calls", "endpoint", "key_vault"];
                fields.iter().any(|f| had.get(*f) != body.get(*f))
            }
        }
    };
    let Some(first) = served.first() else { return out };
    let rule = serde_json::json!({ "size": "standard", "permit": "quick standard deep", "by": SEEDED_BY });
    if writable("model.rule", "default", &rule) {
        out.push(("model.rule".to_string(), "default".to_string(), rule));
    }
    let with_tools = served.iter().find(|m| m.tools);
    for mode in ["think", "tools", "do"] {
        let (model, tools) = if mode == "think" { (first, false) } else if let Some(m) = with_tools { (m, true) } else { continue };
        for size in ["quick", "standard", "deep"] {
            let entity = format!("{mode}.{size}");
            let body = serde_json::json!({ "model": model.id, "tool_calls": tools, "input_micros_per_1k": 0, "output_micros_per_1k": 0, "endpoint": endpoint, "key_vault": key_vault, "by": SEEDED_BY });
            if writable("model.category", &entity, &body) {
                out.push(("model.category".to_string(), entity, body));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOICE: &str = r#"
[dna]
needs = ["memory", "nerves"]

[dna.provides.models]
service = "api"
key = "tenant-key"

[dna.secrets.tenant-key]
kind = "drawn"
family = true
readers = ["api"]
"#;

    #[test]
    fn a_models_provider_is_read_with_its_defaults() {
        let m = declared(VOICE).unwrap().unwrap();
        assert_eq!(m, Models { service: "api".into(), path: "/v1/chat/completions".into(), models_path: "/v1/models".into(), key: "tenant-key".into(), extension: None, grant: None });
        assert_eq!(lines(&Some(m)), "models\tapi\t/v1/chat/completions\t/v1/models\ttenant-key\t\t\n");
        // the provider's own fields, and its words for a tenant served nothing
        let own = VOICE.replace("key = \"tenant-key\"\n\n", "key = \"tenant-key\"\nextension = \"voice\"\ngrant = \"grant tenant-{organism} accounts in voice's admin UI (Projects)\"\n\n");
        let m = declared(&own).unwrap().unwrap();
        assert_eq!((m.extension.as_deref(), m.grant.as_deref()), (Some("voice"), Some("grant tenant-{organism} accounts in voice's admin UI (Projects)")));
        assert!(lines(&Some(m)).ends_with("\ttenant-key\tvoice\tgrant tenant-{organism} accounts in voice's admin UI (Projects)\n"));
        assert!(declared(&VOICE.replace("key = \"tenant-key\"\n\n", "key = \"tenant-key\"\nextension = \"Voice X\"\n\n")).unwrap_err().contains("extension `Voice X`"));
        assert!(declared(&VOICE.replace("key = \"tenant-key\"\n\n", "key = \"tenant-key\"\ngrant = \"a\\nb\"\n\n")).unwrap_err().contains("grant: one line"));
        assert_eq!(declared("[dna]\nneeds = []\n").unwrap(), None);
        assert_eq!(lines(&None), "");
        let paths = VOICE.replace("key = \"tenant-key\"\n\n", "key = \"tenant-key\"\npath = \"/openai/v1/chat\"\nmodels_path = \"/openai/v1/models\"\n\n");
        let m = declared(&paths).unwrap().unwrap();
        assert_eq!((m.path.as_str(), m.models_path.as_str()), ("/openai/v1/chat", "/openai/v1/models"));
    }

    #[test]
    fn each_refusal_says_what_is_wrong() {
        let refuse = |toml: &str| declared(toml).unwrap_err();
        assert!(refuse(&VOICE.replace("[dna.provides.models]", "[dna.provides.search]")).contains("provides models alone"));
        assert!(refuse(&VOICE.replace("service = \"api\"", "service = \"api\"\nport = 8080")).contains("has no `port`"));
        assert!(refuse(&VOICE.replace("service = \"api\"\n", "")).contains("names its service"));
        assert!(refuse(&VOICE.replace("service = \"api\"", "service = \"my api\"")).contains("service `my api`"));
        assert!(refuse(&VOICE.replace("service = \"api\"", "service = 1")).contains("service is a string"));
        assert!(refuse(&VOICE.replace("key = \"tenant-key\"", "key = \"tenant-key\"\npath = \"v1/chat\"")).contains("path `v1/chat`"));
        assert!(refuse(&VOICE.replace("key = \"tenant-key\"", "key = \"tenant-key\"\nmodels_path = \"/v1/../admin\"")).contains("models_path `/v1/../admin`"));
        assert!(refuse(&VOICE.replace("key = \"tenant-key\"", "key = \"tenant-key\"\npath = \"/v1/chat?x=1\"")).contains("path `/v1/chat?x=1`"));
        assert!(refuse(&VOICE.replace("key = \"tenant-key\"\n", "")).contains("names its key"));
        assert!(refuse(&VOICE.replace("key = \"tenant-key\"", "key = \"other\"")).contains("declares no secret `other`"));
        // a key that is not a drawn family: a slot family, a drawn single
        let slot = VOICE.replace("kind = \"drawn\"", "kind = \"slot\"");
        assert!(refuse(&slot).contains("a drawn family"));
        let single = VOICE.replace("family = true\n", "");
        assert!(refuse(&single).contains("a drawn family"));
        assert!(refuse("[dna]\nprovides = 1\n").contains("table of capabilities"));
        assert!(refuse("[dna.provides]\nmodels = 1\n").contains("[dna.provides.models] is a table"));
    }

    #[test]
    fn a_tenants_label_is_its_organisms_name_when_the_rule_takes_it() {
        assert_eq!(tenant_label("voice").unwrap(), "voice");
        assert_eq!(tenant_label("todo-2").unwrap(), "todo-2");
        assert!(tenant_label("Todo").unwrap_err().contains("cannot label its key"));
        assert!(tenant_label("my_app").unwrap_err().contains("`my_app`"));
        assert!(tenant_label("-x").is_err() && tenant_label("").is_err());
    }

    #[test]
    fn a_models_list_is_read_in_its_order_and_tools_unsaid_is_none() {
        let s = served(r#"{"object": "list", "data": [{"id": "qwen3-0.6b", "object": "model", "capabilities": {"tools": false}}, {"id": "small"}, {"id": "coder", "capabilities": {"tools": true}}, {"object": "model"}]}"#, None).unwrap();
        assert_eq!(s, vec![Served { id: "qwen3-0.6b".into(), tools: false }, Served { id: "small".into(), tools: false }, Served { id: "coder".into(), tools: true }]);
        assert_eq!(served(r#"{"data": []}"#, None).unwrap(), vec![]);
        assert!(served("not json", None).unwrap_err().contains("not JSON"));
        assert!(served(r#"{"models": []}"#, None).unwrap_err().contains("no `data` array"));
        // a model that says it is not available to this key is not served
        assert_eq!(served(r#"{"data": [{"id": "a", "available": false}, {"id": "b"}]}"#, None).unwrap(), vec![Served { id: "b".into(), tools: false }]);
    }

    #[test]
    fn a_providers_own_fields_are_read_from_its_extension() {
        // voice's shape: its fields under `voice`, the whole catalog listed
        let body = r#"{"data": [
            {"id": "big", "voice": {"available": false, "capabilities": {"tools": true}}},
            {"id": "qwen3-0.6b", "voice": {"available": true, "capabilities": {"tools": false}}},
            {"id": "coder", "voice": {"available": true, "capabilities": {"tools": true}}},
            {"id": "top", "available": false, "capabilities": {"tools": true}}
        ]}"#;
        let s = served(body, Some("voice")).unwrap();
        assert_eq!(s, vec![Served { id: "qwen3-0.6b".into(), tools: false }, Served { id: "coder".into(), tools: true }, Served { id: "top".into(), tools: false }]);
        // none available to the key: served nothing
        assert_eq!(served(r#"{"data": [{"id": "a", "voice": {"available": false}}]}"#, Some("voice")).unwrap(), vec![]);
    }

    #[test]
    fn a_tenant_served_nothing_is_told_what_to_do_in_the_providers_words() {
        assert_eq!(
            serve_none("voice", "todo", "home", Some("grant tenant-{organism} accounts in voice's admin UI (Projects)"), None),
            "the habitat's models (voice) serve none for todo yet: grant tenant-todo accounts in voice's admin UI (Projects), then `hale habitat models todo`"
        );
        // the habitat's name, as its services are handed it, beside the tenant's
        assert_eq!(
            serve_none("voice", "habitat", "home", Some("grant {habitat}-{organism}'s project accounts in voice's admin UI (Projects)"), None),
            "the habitat's models (voice) serve none for habitat yet: grant home-habitat's project accounts in voice's admin UI (Projects), then `hale habitat models habitat`"
        );
        assert_eq!(
            serve_none("voice", "todo", "home", Some("grant {habitat}-habitat's project accounts, then {habitat}-{organism}'s"), None),
            "the habitat's models (voice) serve none for todo yet: grant home-habitat's project accounts, then home-todo's, then `hale habitat models todo`"
        );
        assert_eq!(serve_none("voice", "todo", "home", None, Some("http://x answered 502")), "the habitat's models (voice) serve none for todo yet (http://x answered 502): `hale habitat models todo` maps them once one is served");
    }

    fn latest(rows: &[(&str, &str, serde_json::Value)]) -> BTreeMap<(String, String), serde_json::Value> {
        rows.iter().map(|(k, e, b)| ((k.to_string(), e.to_string()), b.clone())).collect()
    }

    #[test]
    fn the_map_is_seeded_from_what_is_served() {
        const EP: &str = "http://10.0.0.5:8080/v1/chat/completions";
        let served_ = vec![Served { id: "qwen3-0.6b".into(), tools: false }, Served { id: "coder".into(), tools: true }];
        let rows = seed_rows(&served_, &BTreeMap::new(), EP, "model-HABITAT_ABC");
        let names: Vec<String> = rows.iter().map(|(k, e, _)| format!("{k} {e}")).collect();
        assert_eq!(names, ["model.rule default", "model.category think.quick", "model.category think.standard", "model.category think.deep", "model.category tools.quick", "model.category tools.standard", "model.category tools.deep", "model.category do.quick", "model.category do.standard", "model.category do.deep"]);
        assert_eq!(rows[0].2, serde_json::json!({"size": "standard", "permit": "quick standard deep", "by": "hale habitat"}));
        let think = &rows[2].2;
        assert_eq!((think["model"].as_str(), think["tool_calls"].as_bool(), think["endpoint"].as_str(), think["key_vault"].as_str(), think["by"].as_str()), (Some("qwen3-0.6b"), Some(false), Some(EP), Some("model-HABITAT_ABC"), Some("hale habitat")));
        let tools = &rows[8].2;
        assert_eq!((tools["model"].as_str(), tools["tool_calls"].as_bool()), (Some("coder"), Some(true)));

        // no model returns tool calls: think alone, and no tools or do row
        let plain = vec![Served { id: "qwen3-0.6b".into(), tools: false }];
        let rows = seed_rows(&plain, &BTreeMap::new(), EP, "k");
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert!(rows.iter().all(|(_, e, _)| e == "default" || e.starts_with("think.")));

        // nothing served: nothing seeded
        assert!(seed_rows(&[], &BTreeMap::new(), EP, "k").is_empty());
    }

    #[test]
    fn a_row_a_person_wrote_is_kept_and_the_habitats_own_is_not_repeated() {
        const EP: &str = "http://h:1/v1/chat/completions";
        let served_ = vec![Served { id: "m".into(), tools: false }];
        let first = seed_rows(&served_, &BTreeMap::new(), EP, "k");
        // the same rows again, as the record holds them: nothing to write
        let held = latest(&first.iter().map(|(k, e, b)| (k.as_str(), e.as_str(), b.clone())).collect::<Vec<_>>());
        assert!(seed_rows(&served_, &held, EP, "k").is_empty());
        // another served model: the habitat's own rows move with it
        let moved = seed_rows(&[Served { id: "m2".into(), tools: false }], &held, EP, "k");
        assert_eq!(moved.len(), 3, "the three think rows, not the rule: {moved:?}");
        // a person's rule and category, and init's, are kept
        let person = latest(&[
            ("model.rule", "default", serde_json::json!({"size": "deep", "permit": "", "by": "riley"})),
            ("model.category", "think.standard", serde_json::json!({"model": "theirs", "by": "riley"})),
            ("model.category", "think.deep", serde_json::json!({"model": "claude-opus-5", "by": "hale habitat graft"})),
        ]);
        let rows = seed_rows(&served_, &person, EP, "k");
        let names: Vec<&str> = rows.iter().map(|(_, e, _)| e.as_str()).collect();
        assert_eq!(names, ["think.quick"], "{rows:?}");
    }
}
