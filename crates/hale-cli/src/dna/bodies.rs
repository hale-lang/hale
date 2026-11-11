//! The bodies of the habitat's grafted organisms.
//!
//! An organism grafted from a sapling lives in the habitat's own store
//! (`<habitat>/saplings/<name>`), where no person runs its body: the habitat
//! keeps it, as it keeps its own. Its body is an allocation like its memory
//! and nerves, `habitat.allocated <key>/body`, on the habitat's own machine:
//! a compose service of the habitat's project, `body-<key>`, beside the
//! habitat's `body`, from the same image, user, network and mounts, running
//! `hale dna dev <root> --no-iris`. `dev` rather than `run`: it derives the
//! organism's memory, nerves and handover from its allocation at each start
//! (`run` takes them from its environment), so a body restarted after a
//! migration or a move reaches them where they are now, and it is what the
//! habitat's own body runs for the same reason.
//!
//! What the service needs of this machine (the image, the user, the network,
//! the mounts, the environment) is the habitat body's, which only `up`
//! knows: `up` keeps it beside the habitat (`body-shape.json`), and each
//! organism's `body.yaml` is rendered from it, at `up`, at a graft and at an
//! apply. A habitat that runs no body of its own (`up --no-body`) keeps no
//! shape, and its organisms get none either.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// The compose file of an organism's body, beside its allocations.
pub(super) const BODY_FILE: &str = "body.yaml";

/// What an organism's body service takes of the habitat's own body: the
/// parts of its compose service that are this machine's, not the genome's.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
pub(super) struct BodyShape {
    pub image: String,
    /// the `user:` and `group_add:` lines ("" on macOS)
    pub user: String,
    /// the `network_mode:` (Linux) or `extra_hosts:` (macOS) lines
    pub network: String,
    /// the host's name, given the container on a project network (macOS),
    /// so the lease a body takes names the machine `hale dna status` asks
    /// from; "" on the host's network, whose name the container has
    pub hostname: String,
    /// the `env_file:` lines ("" unless the backends are the operator's)
    pub env_file: String,
    /// the `environment:` entries, a line each
    pub environment: String,
    /// the volumes, `(source, target)`
    pub mounts: Vec<(String, String)>,
    pub stop_grace_secs: u64,
    /// the habitat face's `environment:` entries less its own state and its
    /// published identity: what an organism's face takes ("" kept by an
    /// earlier `up`, whose organisms then get no face until the next)
    #[serde(default)]
    pub face_environment: String,
    /// the address its head block is published on (macOS, a project
    /// network); "" on the host's network, where it binds its ports itself
    #[serde(default)]
    pub face_publish: String,
}

impl BodyShape {
    /// Whether `path` is under a directory mounted at its own path.
    fn covers(&self, path: &str) -> bool {
        self.mounts.iter().any(|(s, t)| s == t && (path == s || path.starts_with(&format!("{}/", s.trim_end_matches('/')))))
    }
}

/// The compose service of the body of the organism keyed `key`.
pub(super) fn service_of(key: &str) -> String {
    format!("body-{key}")
}

/// The compose service of the face (and head) of the organism keyed `key`.
pub(super) fn face_service_of(key: &str) -> String {
    format!("face-{key}")
}

/// The port its graft recorded for an organism's face (`face` of its
/// `habitat.grafted` row, `http://<address>:<port>`): the first of its head
/// block, the head the next.
pub(super) fn face_port_of(face: &str) -> Option<u16> {
    face.trim_end_matches('/').rsplit_once(':').and_then(|(_, p)| p.parse().ok())
}

fn shape_path() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("body-shape.json")
}

/// The habitat body's shape kept for its organisms' bodies (`up`).
pub(super) fn keep_shape(shape: &BodyShape) -> Result<(), String> {
    let path = shape_path();
    let text = serde_json::to_string_pretty(shape).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{text}\n")).map_err(|e| format!("write {}: {e}", path.display()))
}

/// The shape `up` kept, `None` when the habitat runs no body of its own.
pub(super) fn shape() -> Option<BodyShape> {
    std::fs::read_to_string(shape_path()).ok().and_then(|t| serde_json::from_str(&t).ok())
}

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The compose file of the body of the organism keyed `key` at `root`, its
/// service labelled with `labels` (`compose_labels(<organism>, "body")`).
pub(super) fn render(shape: &BodyShape, key: &str, labels: &str, root: &str) -> String {
    render_with_face(shape, key, labels, "", root, None, "")
}

/// `render`, and when `face_port` names its graft's head block (and the
/// shape carries the face's environment), its face beside it: a service
/// `face-<key>` running `hale dna head <root>` as the habitat's face runs
/// its own, its state in `face_state`, signing in through the habitat's
/// identity (it publishes none of its own).
pub(super) fn render_with_face(shape: &BodyShape, key: &str, labels: &str, face_labels: &str, root: &str, face_port: Option<u16>, face_state: &str) -> String {
    let mut s = format!("# The body of the organism {key}, kept by the habitat: written by `hale habitat up`,\n# a graft and an apply from the habitat body's shape (body-shape.json).\nservices:\n  {}:\n", service_of(key));
    s.push_str(&format!("    image: {}\n", q(&shape.image)));
    s.push_str(labels);
    s.push_str("    init: true\n");
    s.push_str(&shape.user);
    s.push_str(&shape.network);
    if !shape.hostname.is_empty() {
        s.push_str(&format!("    hostname: {}\n", q(&shape.hostname)));
    }
    s.push_str(&format!("    working_dir: {}\n", q(root)));
    s.push_str(&shape.env_file);
    s.push_str("    environment:\n");
    s.push_str(&shape.environment);
    // the habitat's body serves the readings' fixed ports on this machine;
    // a second body there would take them from it (or lose them to it)
    s.push_str("      HALE_DNA_SENSES_PORT: \"0\"\n");
    s.push_str("    volumes:\n");
    for (src, dst) in &shape.mounts {
        s.push_str(&format!("      - {}\n", q(&format!("{src}:{dst}"))));
    }
    if !shape.covers(root) {
        s.push_str(&format!("      - {}\n", q(&format!("{root}:{root}"))));
    }
    s.push_str(&format!("    command: [\"dna\", \"dev\", {}, \"--no-iris\"]\n", q(root)));
    // as the habitat's body: its clean exit is a drain the habitat asked for
    s.push_str("    restart: on-failure\n");
    s.push_str(&format!("    stop_grace_period: {}s\n", shape.stop_grace_secs));
    if let Some(base) = face_port.filter(|_| !shape.face_environment.is_empty()) {
        s.push_str(&format!("  {}:\n", face_service_of(key)));
        s.push_str(&format!("    image: {}\n", q(&shape.image)));
        s.push_str(face_labels);
        s.push_str("    init: true\n");
        s.push_str(&shape.user);
        s.push_str(&shape.network);
        if !shape.hostname.is_empty() {
            s.push_str(&format!("    hostname: {}\n", q(&shape.hostname)));
        }
        s.push_str(&format!("    working_dir: {}\n", q(root)));
        s.push_str(&shape.env_file);
        s.push_str("    environment:\n");
        s.push_str(&shape.face_environment);
        s.push_str(&format!("      HALE_DNA_HEAD_STATE: {}\n", q(face_state)));
        if !shape.face_publish.is_empty() {
            // its head block, as the habitat's face publishes its own
            s.push_str("    ports:\n");
            for offset in (0..HEAD_BLOCK).filter(|o| *o != readings::GRAFANA_OFFSET) {
                let port = base + offset;
                s.push_str(&format!("      - {}\n", q(&format!("{}:{port}:{port}", shape.face_publish))));
            }
        }
        s.push_str("    volumes:\n");
        for (src, dst) in &shape.mounts {
            s.push_str(&format!("      - {}\n", q(&format!("{src}:{dst}"))));
        }
        if !shape.covers(root) {
            s.push_str(&format!("      - {}\n", q(&format!("{root}:{root}"))));
        }
        if !shape.covers(face_state) {
            s.push_str(&format!("      - {}\n", q(&format!("{face_state}:{face_state}"))));
        }
        s.push_str(&format!("    command: [\"dna\", \"head\", {}]\n", q(root)));
        s.push_str("    restart: on-failure\n");
    }
    s
}

/// An organism the habitat keeps a body for.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Kept {
    pub key: String,
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    /// the first port of its head block, as its graft recorded it
    pub face_port: Option<u16>,
}

/// The organisms the habitat keeps bodies for, of the habitat's record
/// `rows`: each live graft of a sapling (an organism in the habitat's own
/// store, which no person runs), at the directory `root_of` its key names.
/// A directory grafted from a checkout is the person's: its body is theirs.
pub(super) fn kept_in(rows: &[serde_json::Value], root_of: &dyn Fn(&str, &str) -> Option<PathBuf>) -> Vec<Kept> {
    let mut latest: std::collections::BTreeMap<String, serde_json::Value> = std::collections::BTreeMap::new();
    for r in rows.iter().filter(|r| r["kind"] == "habitat.grafted") {
        if let Some(e) = r["entity"].as_str() {
            latest.insert(e.to_string(), r["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_default());
        }
    }
    latest
        .into_iter()
        .filter(|(_, b)| b["state"].as_str() != Some("removed"))
        .filter_map(|(key, b)| {
            let sapling = b["sapling"].as_str().filter(|s| !s.is_empty())?;
            let id = b["child"].as_str().filter(|c| c.starts_with(&key))?.to_string();
            let root = root_of(&key, sapling)?;
            let name = b["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(sapling).to_string();
            let face_port = b["face"].as_str().and_then(face_port_of);
            Some(Kept { key, id, name, root, face_port })
        })
        .collect()
}

fn allocation_dir_of(key: &str) -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("allocations").join(key)
}

/// The directory of the sapling organism keyed `key` on this machine: its
/// allocation's cache, else its store; `None` when neither is here.
fn root_here(key: &str, sapling: &str) -> Option<PathBuf> {
    let store = PathBuf::from(local_habitat_dir()).join("saplings").join(sapling);
    let cached = std::fs::read_to_string(allocation_dir_of(key).join("allocation.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v["root"].as_str().map(PathBuf::from));
    cached.filter(|r| r.starts_with(&store)).or(Some(store)).filter(|r| r.join("hale.toml").is_file())
}

/// The `habitat.allocated <key>/body` row's body.
pub(super) fn allocation_body(id: &str, key: &str, project: &str) -> serde_json::Value {
    allocation_of(id, project, &service_of(key))
}

/// The `habitat.allocated <key>/<part>` row's body for a service of the
/// habitat's project that holds no data (a body, a face): role `body`, so a
/// removal stops and removes it with the rest of the project's.
fn allocation_of(id: &str, project: &str, service: &str) -> serde_json::Value {
    serde_json::json!({ "organism": id, "role": "body", "service": service, "project": project, "provider": "compose", "host": "local", "vault": [] })
}

/// Whether the record `rows` hold a live `<key>/body` allocation.
pub(super) fn recorded(rows: &[serde_json::Value], key: &str) -> bool {
    recorded_part(rows, key, "body")
}

fn recorded_part(rows: &[serde_json::Value], key: &str, part: &str) -> bool {
    let entity = format!("{key}/{part}");
    rows.iter()
        .filter(|r| r["kind"] == "habitat.allocated" && r["entity"] == entity.as_str())
        .last()
        .is_some_and(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).is_some_and(|b| b["state"].as_str() != Some("removed")))
}

/// The body of `k` recorded when the record lacks it (an organism grafted
/// before bodies were kept gets its allocation here) and its compose file
/// written from `shape`.
fn ensure(k: &Kept, rows: &[serde_json::Value], shape: &BodyShape) -> Result<(), String> {
    if !recorded(rows, &k.key) {
        sapling::record(&[serde_json::json!({ "kind": "habitat.allocated", "entity": format!("{}/body", k.key), "body": allocation_body(&k.id, &k.key, &local_habitat_project()).to_string() })])?;
    }
    let dir = allocation_dir_of(&k.key);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    // its face and head, on the block its graft recorded, beside its body
    let face_port = k.face_port.filter(|_| !shape.face_environment.is_empty());
    let face_state = dir.join("face-state");
    if face_port.is_some() {
        std::fs::create_dir_all(&face_state).map_err(|e| format!("create {}: {e}", face_state.display()))?;
        if !recorded_part(rows, &k.key, "face") {
            sapling::record(&[serde_json::json!({ "kind": "habitat.allocated", "entity": format!("{}/face", k.key), "body": allocation_of(&k.id, &local_habitat_project(), &face_service_of(&k.key)).to_string() })])?;
        }
    }
    let text = render_with_face(shape, &k.key, &compose_labels(&k.name, "body"), &compose_labels(&k.name, "face"), &k.root.display().to_string(), face_port, &face_state.display().to_string());
    let path = dir.join(BODY_FILE);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// The body of the organism keyed `key` brought up from its compose file
/// (`recreate`: made again, as at an upgrade, so it starts on the tree as
/// it is now). Not waited on: a body comes up while it builds its genome.
pub(super) fn start(key: &str, recreate: bool) -> Result<(), String> {
    let dir = allocation_dir_of(key);
    let file = dir.join(BODY_FILE);
    if !file.is_file() {
        return Err(format!("{} is not written: `hale habitat up` writes it", file.display()));
    }
    let service = service_of(key);
    let mut cmd = Command::new("env");
    cmd.args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"]).arg(local_habitat_project()).arg("--project-directory").arg(&dir).arg("-f").arg(&file).args(["up", "-d", "--no-deps"]);
    if recreate {
        cmd.arg("--force-recreate");
    }
    cmd.arg(&service);
    // its face, when its compose file holds one
    let face = face_service_of(key);
    if std::fs::read_to_string(&file).unwrap_or_default().contains(&format!("\n  {face}:\n")) {
        cmd.arg(&face);
    }
    let ran = cmd.output().map_err(|e| format!("docker compose: {e}"))?;
    if !ran.status.success() {
        return Err(format!("docker compose up -d {service}: {}", String::from_utf8_lossy(&ran.stderr).trim()));
    }
    Ok(())
}

/// What became of one organism's body.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Outcome {
    pub key: String,
    pub name: String,
    pub ok: bool,
    pub why: String,
    /// `up` while its container runs, else `down`
    pub state: String,
    pub detail: String,
}

impl Outcome {
    /// The line `up` prints for it.
    pub(super) fn line(&self) -> String {
        if self.ok {
            format!("{:<8}{}/body of {} {} (compose service {}{})", "body", self.key, self.name, self.state, service_of(&self.key), if self.detail.is_empty() { String::new() } else { format!(", {}", self.detail) })
        } else {
            format!("{:<8}{}/body of {} did not come up: {}", "note", self.key, self.name, self.why)
        }
    }

    /// Its entry in an apply's reply, as `Commands::apply_allocations` lists one.
    pub(super) fn applied(&self) -> serde_json::Value {
        serde_json::json!({ "allocation": format!("{}/body", self.key), "host": shown_host("local"), "provider": "compose", "ok": self.ok, "why": self.why, "state": self.state, "detail": self.detail })
    }
}

fn outcome(k: &Kept, done: Result<(), String>) -> Outcome {
    match done {
        Ok(()) => {
            let seen = running(&k.key);
            Outcome { key: k.key.clone(), name: k.name.clone(), ok: true, why: String::new(), state: if seen.is_some() { "up" } else { "down" }.to_string(), detail: seen.unwrap_or_default() }
        }
        Err(why) => Outcome { key: k.key.clone(), name: k.name.clone(), ok: false, why, state: "down".to_string(), detail: String::new() },
    }
}

/// The organisms the habitat keeps bodies for, as its record holds them.
fn kept() -> (Vec<serde_json::Value>, Vec<Kept>) {
    let rows = rows_of(&genome_dir());
    let kept = kept_in(&rows, &root_here);
    (rows, kept)
}

/// Every kept organism's body recorded, written and brought up (`up` and an
/// apply on this machine); nothing while the habitat runs no body of its own.
pub(super) fn keep_all() -> Vec<Outcome> {
    let Some(shape) = shape() else { return Vec::new() };
    let (rows, kept) = kept();
    kept.iter().map(|k| outcome(k, ensure(k, &rows, &shape).and_then(|()| start(&k.key, false)))).collect()
}

/// After a sapling's graft: its body recorded, written and brought up, the
/// lines that say so. A habitat with no body of its own says that instead.
pub(super) fn after_graft(root: &Path) -> Result<Vec<String>, String> {
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().ok_or("the organism pins no record")?;
    let key = id[..12.min(id.len())].to_string();
    let Some(shape) = shape() else {
        return Ok(vec![format!("{:<8}its body ({key}/body) is not started: the habitat runs no body of its own (`hale habitat up` starts both)", "note")]);
    };
    let (rows, kept) = kept();
    let Some(k) = kept.into_iter().find(|k| k.key == key) else {
        return Err(format!("the habitat's record holds no graft of a sapling keyed {key}"));
    };
    super::progress(&format!("starting {}'s body (compose service {})", k.name, service_of(&key)));
    let o = outcome(&k, ensure(&k, &rows, &shape).and_then(|()| start(&key, false)));
    if !o.ok {
        return Err(format!("its body ({key}/body) did not come up: {}; `hale habitat apply` brings it up", o.why));
    }
    Ok(vec![format!("{:<8}its body ({key}/body) on {}: compose service {}, `hale dna dev` on its directory", "body", shown_host("local"), service_of(&key))])
}

/// The body of the organism keyed `key` made again on its tree as it is
/// now (an upgrade): `None` when the habitat keeps no body for it.
pub(super) fn restart(key: &str) -> Option<Result<String, String>> {
    let shape = shape()?;
    let (rows, kept) = kept();
    let k = kept.into_iter().find(|k| k.key == key)?;
    Some(ensure(&k, &rows, &shape).and_then(|()| start(key, true)).map(|()| format!("{:<8}{key}/body restarted on the new revision (compose service {})", "body", service_of(key))))
}

/// The state of the body of `key` while it runs, `None` otherwise.
pub(super) fn running(key: &str) -> Option<String> {
    service_status(&service_of(key))
}

/// The organisms' bodies running (or between restarts) in the habitat's
/// project, by container id: what `down` stops, drained, before the rest.
pub(super) fn running_ids() -> Vec<(String, String)> {
    let Docker::Answered(out) = docker(&[
        "ps",
        "--filter",
        &format!("label=com.docker.compose.project={}", local_habitat_project()),
        "--filter",
        "label=hale.role=body",
        "--filter",
        "status=running",
        "--filter",
        "status=restarting",
        "--format",
        "{{.ID}} {{.Label \"com.docker.compose.service\"}}",
    ]) else {
        return Vec::new();
    };
    organism_bodies(&out)
}

/// Of `docker ps` lines (`<id> <service>`), the organisms' bodies: every
/// `body-<key>`, never the habitat's own `body`.
pub(super) fn organism_bodies(listed: &str) -> Vec<(String, String)> {
    listed.lines().filter_map(|l| l.trim().split_once(' ')).filter(|(_, s)| s.starts_with("body-")).map(|(i, s)| (i.to_string(), s.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> BodyShape {
        BodyShape {
            image: "hale-habitat:abc".to_string(),
            user: "    user: \"1000:1000\"\n    group_add:\n      - \"998\"\n".to_string(),
            network: "    network_mode: \"host\"\n".to_string(),
            hostname: String::new(),
            env_file: String::new(),
            environment: "      HOME: \"/h\"\n      HALE_DNA_HABITAT: \"/h\"\n      HALE_DNA_COMPOSE: \"off\"\n".to_string(),
            mounts: vec![("/h".to_string(), "/h".to_string()), ("/run/docker.sock".to_string(), "/var/run/docker.sock".to_string())],
            stop_grace_secs: 45,
            face_environment: String::new(),
            face_publish: String::new(),
        }
    }

    #[test]
    fn a_body_runs_dna_dev_on_its_organism_with_the_habitats_shape() {
        let labels = "    labels:\n      hale.habitat: \"lab\"\n      hale.organism: \"voice\"\n      hale.role: \"body\"\n";
        let text = render(&shape(), "0a1b2c3d4e5f", labels, "/h/saplings/voice");
        assert!(text.contains("services:\n  body-0a1b2c3d4e5f:\n    image: \"hale-habitat:abc\"\n"), "{text}");
        assert!(text.contains(labels), "labelled as the organism's body: {text}");
        assert!(text.contains("    working_dir: \"/h/saplings/voice\"\n"), "{text}");
        assert!(text.contains("    command: [\"dna\", \"dev\", \"/h/saplings/voice\", \"--no-iris\"]\n"), "{text}");
        assert!(text.contains("    restart: on-failure\n    stop_grace_period: 45s\n"), "as the habitat's body: {text}");
        assert!(text.contains("    user: \"1000:1000\"\n") && text.contains("    network_mode: \"host\"\n"), "{text}");
        assert!(text.contains("      HALE_DNA_COMPOSE: \"off\"\n") && text.contains("      HALE_DNA_HABITAT: \"/h\"\n"), "the habitat's environment: {text}");
        assert!(text.contains("      HALE_DNA_SENSES_PORT: \"0\"\n"), "the readings' ports are the habitat body's: {text}");
        assert!(text.contains("      - \"/h:/h\"\n") && text.contains("      - \"/run/docker.sock:/var/run/docker.sock\"\n"), "{text}");
        // its root is under the habitat's directory: mounted once
        assert_eq!(text.matches("/h/saplings/voice:").count(), 0, "{text}");
        assert!(!text.contains("hostname:") && !text.contains("depends_on") && !text.contains("build:"), "no build, no dependency across files: {text}");
    }

    #[test]
    fn a_root_outside_the_mounts_is_mounted_and_a_project_network_names_the_host() {
        let mut s = shape();
        s.hostname = "box".to_string();
        s.network = "    extra_hosts:\n      - \"host.docker.internal:host-gateway\"\n".to_string();
        let text = render(&s, "0a1b2c3d4e5f", "", "/elsewhere/voice");
        assert!(text.contains("      - \"/elsewhere/voice:/elsewhere/voice\"\n"), "{text}");
        assert!(text.contains("    hostname: \"box\"\n"), "{text}");
        assert!(!shape().covers("/hx/voice") && shape().covers("/h") && shape().covers("/h/a/b"));
    }

    #[test]
    fn bodies_are_kept_for_live_sapling_grafts_alone() {
        let row = |e: &str, body: serde_json::Value| serde_json::json!({ "kind": "habitat.grafted", "entity": e, "body": body.to_string() });
        let rows = vec![
            row("aaaaaaaaaaaa", serde_json::json!({ "child": "aaaaaaaaaaaa11", "name": "voice", "sapling": "voice", "face": "http://127.0.0.1:9024" })),
            // a directory grafted from a checkout: the person runs its body
            row("bbbbbbbbbbbb", serde_json::json!({ "child": "bbbbbbbbbbbb22", "name": "app" })),
            row("cccccccccccc", serde_json::json!({ "child": "cccccccccccc33", "name": "gone", "sapling": "gone" })),
            row("cccccccccccc", serde_json::json!({ "child": "cccccccccccc33", "name": "gone", "sapling": "gone", "state": "removed" })),
            // a sapling not on this machine
            row("dddddddddddd", serde_json::json!({ "child": "dddddddddddd44", "name": "far", "sapling": "far" })),
        ];
        let root_of = |_: &str, s: &str| (s != "far").then(|| PathBuf::from(format!("/h/saplings/{s}")));
        let kept = kept_in(&rows, &root_of);
        assert_eq!(kept, vec![Kept { key: "aaaaaaaaaaaa".to_string(), id: "aaaaaaaaaaaa11".to_string(), name: "voice".to_string(), root: PathBuf::from("/h/saplings/voice"), face_port: Some(9024) }]);
    }

    #[test]
    fn an_organism_s_face_runs_beside_its_body_on_its_recorded_block() {
        let mut s = shape();
        s.face_environment = "      HALE_DNA_LISTEN_HOST: \"0.0.0.0\"\n".to_string();
        s.face_publish = "127.0.0.1".to_string();
        let text = render_with_face(&s, "0a1b2c3d4e5f", "", "", "/h/saplings/voice", Some(9024), "/h/allocations/0a1b2c3d4e5f/face-state");
        assert!(text.contains("\n  body-0a1b2c3d4e5f:\n") && text.contains("\n  face-0a1b2c3d4e5f:\n"), "{text}");
        let face = text.split("\n  face-0a1b2c3d4e5f:\n").nth(1).unwrap();
        assert!(face.contains("command: [\"dna\", \"head\", \"/h/saplings/voice\"]"), "{face}");
        assert!(face.contains("HALE_DNA_HEAD_STATE: \"/h/allocations/0a1b2c3d4e5f/face-state\""), "{face}");
        assert!(face.contains("\"127.0.0.1:9024:9024\"") && face.contains("\"127.0.0.1:9025:9025\""), "its face and head ports: {face}");
        assert!(!face.contains(&format!(":{}:", 9024 + readings::GRAFANA_OFFSET)), "no Grafana port of its own: {face}");
        assert!(!face.contains("HALE_DNA_IDENTITY"), "it publishes no identity of its own: {face}");
        assert!(!face.contains("HALE_DNA_FACE_HOME"), "it lands on its own organization, not the operator face: {face}");
        // a shape an earlier `up` kept carries no face environment: no face
        let old = render_with_face(&shape(), "0a1b2c3d4e5f", "", "", "/h/saplings/voice", Some(9024), "/x");
        assert!(!old.contains("face-0a1b2c3d4e5f"), "{old}");
        // on the host's network it binds its ports itself
        s.face_publish = String::new();
        assert!(!render_with_face(&s, "0a1b2c3d4e5f", "", "", "/h/saplings/voice", Some(9024), "/x").contains("ports:"));
        assert_eq!(face_port_of("http://127.0.0.1:9024/"), Some(9024));
    }

    #[test]
    fn a_body_allocation_is_recorded_once_and_again_after_a_removal() {
        let alloc = |body: serde_json::Value| serde_json::json!({ "kind": "habitat.allocated", "entity": "aaaaaaaaaaaa/body", "body": body.to_string() });
        let body = allocation_body("aaaaaaaaaaaa11", "aaaaaaaaaaaa", "hale-habitat-local-0000");
        assert_eq!(body["service"], "body-aaaaaaaaaaaa");
        assert_eq!(body["host"], "local");
        assert_eq!(body["provider"], "compose");
        assert!(!recorded(&[], "aaaaaaaaaaaa"));
        assert!(recorded(&[alloc(body.clone())], "aaaaaaaaaaaa"));
        let mut gone = body.clone();
        gone["state"] = serde_json::json!("removed");
        assert!(!recorded(&[alloc(body), alloc(gone)], "aaaaaaaaaaaa"));
    }

    #[test]
    fn down_stops_the_organisms_bodies_not_the_habitats() {
        let listed = "1f2e body\n3c4d body-0a1b2c3d4e5f\n\n5e6f body-ffffffffffff\n";
        assert_eq!(organism_bodies(listed), vec![("3c4d".to_string(), "body-0a1b2c3d4e5f".to_string()), ("5e6f".to_string(), "body-ffffffffffff".to_string())]);
    }

    #[test]
    fn an_applied_body_is_listed_as_the_head_lists_an_allocation() {
        let o = Outcome { key: "0a1b2c3d4e5f".to_string(), name: "voice".to_string(), ok: true, why: String::new(), state: "up".to_string(), detail: "Up 2 seconds".to_string() };
        let v = o.applied();
        assert_eq!(v["allocation"], "0a1b2c3d4e5f/body");
        assert_eq!(v["provider"], "compose");
        assert_eq!(v["state"], "up");
        assert!(o.line().contains("0a1b2c3d4e5f/body of voice up (compose service body-0a1b2c3d4e5f, Up 2 seconds)"), "{}", o.line());
        let failed = Outcome { ok: false, why: "no image".to_string(), state: "down".to_string(), ..o };
        assert!(failed.line().starts_with(&format!("{:<8}0a1b2c3d4e5f/body of voice did not come up: no image", "note")), "{}", failed.line());
    }
}
