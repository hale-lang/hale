//! What of a host's memory the habitat itself holds: the sum of the memory
//! limits (compose's `mem_limit`) of the containers it runs there, handed to
//! every host command as `HALE_DNA_HOST_RESERVED_BYTES`, so a command that
//! sizes itself by the host's memory (a provider's node offering it to
//! models) leaves the habitat's share alone.
//!
//! It is read from the compose files the habitat wrote, under its directory
//! on its own machine (here, or over the person's login through a pointer):
//!
//! - on the habitat's own machine (`local`, or the host the pointer names),
//!   every service of its own allocation's `compose.yaml` (its memory, its
//!   nerves, its senses' store, `body`, `face` and Grafana, which starts on
//!   demand and so is counted whether it runs or not) and its forge's;
//! - on any host, each organism's memory, nerves and senses' store applied
//!   there (`placed.<role>`, else this machine), unless the habitat's own
//!   server holds that part for it (`parts.json` says `reuse`), and its own
//!   services, applied there, from the compose file at its root.
//!
//! A service that declares no limit counts 0, as one whose limit does not
//! read as a size does. On a Mac, docker runs in a VM the Mac gives all of
//! its memory to, whatever its containers use: there the habitat holds the
//! VM's memory (`docker info`'s, read with the host's facts) when it is more
//! than the limits (`hosted::reserved_total`).

use std::collections::BTreeMap;

/// The variable a host command is handed the habitat's share in.
pub(super) const RESERVED_ENV: &str = "HALE_DNA_HOST_RESERVED_BYTES";

/// An organism's parts the habitat allocates as servers of their own, each
/// a compose service `<role>-<key>` of its allocation's file.
const PARTS: &[&str] = &["memory", "nerves", "senses"];

/// What the files of each allocation the script reads are called.
const ALLOCATION_FILES: &[&str] = &["compose.yaml", "parts.json", "placed.memory", "placed.nerves", "placed.senses", "placed.services"];

/// The name an organism's own services' compose file is listed under,
/// beside its allocation's files (it lives at the organism's root).
const SERVICES_COMPOSE: &str = "services.compose.yaml";

/// A memory size as compose reads one (docker's `RAMInBytes`): a number,
/// optionally fractional, an optional space, and an optional unit `k`, `m`,
/// `g`, `t` or `p` (any case, powers of 1024) with an optional `i` and `b`;
/// a bare number is bytes. `None` when it reads as no size.
pub(super) fn mem_bytes(v: &str) -> Option<u64> {
    let s = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, rest) = s.split_at(end);
    if num.is_empty() || !num.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let n: f64 = num.parse().ok()?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest).to_ascii_lowercase();
    let (shift, tail) = match rest.chars().next() {
        Some('k') => (10, &rest[1..]),
        Some('m') => (20, &rest[1..]),
        Some('g') => (30, &rest[1..]),
        Some('t') => (40, &rest[1..]),
        Some('p') => (50, &rest[1..]),
        _ => (0, rest.as_str()),
    };
    let tail = tail.strip_prefix('i').unwrap_or(tail);
    let tail = tail.strip_prefix('b').unwrap_or(tail);
    if !tail.is_empty() {
        return None;
    }
    Some((n * (1u64 << shift) as f64) as u64)
}

/// A YAML scalar without its comment and quotes.
fn scalar(v: &str) -> &str {
    let v = v.split(" #").next().unwrap_or(v).trim();
    v.trim_matches(|c| c == '"' || c == '\'')
}

/// Each service of a compose file, in its order, with its `mem_limit` in
/// bytes (`None` when it declares none, or one that reads as no size).
/// Block-style YAML, as every file the habitat writes is and compose files
/// commonly are: the services are the keys one level under the top-level
/// `services:`, and a service's limit its own `mem_limit:` key.
pub(super) fn compose_limits(text: &str) -> Vec<(String, Option<u64>)> {
    let mut out: Vec<(String, Option<u64>)> = Vec::new();
    let mut in_services = false;
    let (mut service_at, mut key_at): (Option<usize>, Option<usize>) = (None, None);
    for raw in text.lines() {
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = line.len() - t.len();
        if indent == 0 {
            in_services = t.strip_prefix("services:").is_some_and(|r| scalar(r).is_empty());
            (service_at, key_at) = (None, None);
            continue;
        }
        if !in_services {
            continue;
        }
        let at = *service_at.get_or_insert(indent);
        if indent < at {
            continue;
        }
        if indent == at {
            if let Some((name, _)) = t.split_once(':') {
                out.push((scalar(name).to_string(), None));
            }
            key_at = None;
            continue;
        }
        if indent == *key_at.get_or_insert(indent) {
            if let (Some(v), Some(last)) = (t.strip_prefix("mem_limit:"), out.last_mut()) {
                last.1 = mem_bytes(scalar(v));
            }
        }
    }
    out
}

/// The sum of a compose file's services' limits.
fn sum(text: &str) -> u64 {
    compose_limits(text).iter().filter_map(|(_, l)| *l).sum()
}

/// The script, run by `sh` on the habitat's machine, that prints the files
/// the habitat's share is read from, under the habitat's directory `hab`:
/// `entry <path> <bytes>` and the file's bytes after it, a path relative to
/// `hab` (an organism's own services' compose file under its allocation, as
/// `services.compose.yaml`). What is absent is not said.
pub(super) fn files_script(hab: &str) -> String {
    let each = ALLOCATION_FILES.join(" ");
    format!(
        r#"h={h}; say() {{ if [ -f "$2" ]; then echo "entry $1 $(wc -c < "$2" | tr -d ' ')"; cat "$2"; fi; }}; say genome/hale.toml "$h/genome/hale.toml"; say forge/compose.yaml "$h/forge/compose.yaml"; for d in "$h"/allocations/*/; do [ -d "$d" ] || continue; k=$(basename "$d"); for n in {each}; do say "allocations/$k/$n" "$d$n"; done; if [ -f "$d"placed.services ]; then r=$(sed -n 's/.*"root"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d"allocation.json 2>/dev/null | head -n 1); [ -n "$r" ] && say "allocations/$k/{SERVICES_COMPOSE}" "$r/compose.yaml"; fi; done; true"#,
        h = super::hosted::sq(hab),
    )
}

/// What `files_script` printed: each file by its path.
pub(super) fn files_said(out: &str) -> Result<BTreeMap<String, String>, String> {
    Ok(super::hosted::vault_said(out, true)?.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect())
}

/// The habitat's share of `host`'s memory, from its files (`files_said`):
/// its own services' limits when `habitats_machine` (the host is the
/// machine the habitat runs on), and the limits of the organisms'
/// allocations applied on `host`. `(own, organisms)`, in bytes.
pub(super) fn reserved(files: &BTreeMap<String, String>, host: &str, habitats_machine: bool) -> (u64, u64) {
    let file = |p: &str| files.get(p).map(String::as_str);
    let habitat = file("genome/hale.toml").and_then(super::manifest_pin_in_text).filter(|id| id.len() >= 12).map(|id| id[..12].to_string());
    let mut own = 0;
    if habitats_machine {
        if let Some(k) = &habitat {
            own += file(&format!("allocations/{k}/compose.yaml")).map(sum).unwrap_or(0);
        }
        own += file("forge/compose.yaml").map(sum).unwrap_or(0);
    }
    // where a part was applied: a host its `placed.<role>` names, else this
    // machine (an organism's own services have no "else": never applied)
    let on_host = |placed: Option<&str>| -> bool {
        let h = placed.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).and_then(|v| v["host"].as_str().map(str::to_string)).unwrap_or_default();
        h == host || (habitats_machine && (h.is_empty() || h == super::hosted::LOCAL))
    };
    let keys: std::collections::BTreeSet<&str> = files.keys().filter_map(|p| p.strip_prefix("allocations/")?.split_once('/').map(|(k, _)| k)).collect();
    let mut organisms = 0;
    for k in keys.into_iter().filter(|k| habitat.as_deref() != Some(*k)) {
        let parts = file(&format!("allocations/{k}/parts.json")).and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).unwrap_or_default();
        let limits: BTreeMap<String, Option<u64>> = file(&format!("allocations/{k}/compose.yaml")).map(compose_limits).unwrap_or_default().into_iter().collect();
        for role in PARTS {
            // the habitat's own server holds it: no container of its own
            if parts[*role] == "reuse" {
                continue;
            }
            let Some(limit) = limits.get(&format!("{role}-{k}")) else { continue };
            if on_host(file(&format!("allocations/{k}/placed.{role}"))) {
                organisms += limit.unwrap_or(0);
            }
        }
        if let Some(placed) = file(&format!("allocations/{k}/placed.services")) {
            if on_host(Some(placed)) {
                organisms += file(&format!("allocations/{k}/{SERVICES_COMPOSE}")).map(sum).unwrap_or(0);
            }
        }
    }
    (own, organisms)
}

/// A byte count as a person reads it, in MiB.
pub(super) fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes / (1 << 20))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_reads_as_compose_reads_it() {
        assert_eq!(mem_bytes("512m"), Some(512 << 20));
        assert_eq!(mem_bytes("1g"), Some(1 << 30));
        assert_eq!(mem_bytes("128M"), Some(128 << 20));
        assert_eq!(mem_bytes("384MiB"), Some(384 << 20));
        assert_eq!(mem_bytes("2GB"), Some(2 << 30));
        assert_eq!(mem_bytes("64 k"), Some(64 << 10));
        assert_eq!(mem_bytes("1.5g"), Some(3 << 29));
        assert_eq!(mem_bytes("1048576"), Some(1 << 20));
        assert_eq!(mem_bytes("\"256m\""), Some(256 << 20));
        assert_eq!(mem_bytes("'1g'"), Some(1 << 30));
        for none in ["", "m", "lots", "12x", "-1g", "1gg", "${LIMIT}"] {
            assert_eq!(mem_bytes(none), None, "{none}");
        }
    }

    #[test]
    fn each_service_is_read_with_its_own_limit() {
        let text = "# a comment\nservices:\n  memory-k:\n    image: pg\n    mem_limit: 512m\n    environment:\n      mem_limit: 9g\n  api:\n    image: x\n# habitat-body-and-face\n  body:\n    mem_limit: \"1g\"  # a cap\n  face:\n    command: [\"dna\"]\nvolumes:\n  memory-k: {}\n  mem_limit: 5g\n";
        assert_eq!(
            compose_limits(text),
            vec![("memory-k".to_string(), Some(512 << 20)), ("api".to_string(), None), ("body".to_string(), Some(1 << 30)), ("face".to_string(), None)]
        );
        assert_eq!(sum(text), (512 << 20) + (1 << 30));
        // a four-space file reads the same, and a limit no size reads as counts 0
        assert_eq!(compose_limits("services:\n    a:\n        mem_limit: 128m\n    b:\n        mem_limit: ${B}\n"), vec![("a".to_string(), Some(128 << 20)), ("b".to_string(), None)]);
        assert_eq!(sum("version: '3'\n"), 0);
    }

    /// The habitat's files as `files_said` reads them: its own allocation
    /// (`habitat0000a`), its forge, an organism (`organism00b`) whose
    /// memory the habitat's server holds and whose senses' store is placed
    /// on `gpu`, and its services applied on `gpu`.
    fn habitat_files() -> BTreeMap<String, String> {
        let own = "services:\n  memory-habitat0000a:\n    mem_limit: 512m\n  nerves-habitat0000a:\n    mem_limit: 128m\n  senses-habitat0000a:\n    mem_limit: 512m\n# habitat-body-and-face\n  body:\n    image: h\n  face:\n    image: h\n  grafana:\n    mem_limit: 384m\nvolumes:\n  memory-habitat0000a: {}\n";
        let organism = "services:\n  memory-organism00b:\n    mem_limit: 512m\n  nerves-organism00b:\n    mem_limit: 128m\n  senses-organism00b:\n    mem_limit: 512m\n";
        [
            ("genome/hale.toml", "[dna]\nrecord = \"habitat0000a-rest-of-the-id\"\n"),
            ("forge/compose.yaml", "services:\n  forge:\n    mem_limit: 384m\n"),
            ("allocations/habitat0000a/compose.yaml", own),
            ("allocations/habitat0000a/parts.json", "{\"memory\":\"reuse\",\"nerves\":\"reuse\",\"senses\":\"reuse\"}"),
            ("allocations/organism00b/compose.yaml", organism),
            ("allocations/organism00b/parts.json", "{\"memory\":\"reuse\",\"nerves\":\"own\",\"senses\":\"own\"}"),
            ("allocations/organism00b/placed.senses", "{\"host\":\"gpu\",\"provider\":\"ssh\"}\n"),
            ("allocations/organism00b/placed.services", "{\"host\":\"gpu\",\"provider\":\"ssh\"}\n"),
            ("allocations/organism00b/services.compose.yaml", "services:\n  api:\n    mem_limit: 1g\n  worker:\n    image: w\n"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn the_habitats_machine_holds_its_own_services_and_what_runs_there() {
        let files = habitat_files();
        // its own: memory, nerves, senses, Grafana (body and face declare
        // none) and the forge; of the organism, its nerves alone stay here
        let own = (512 + 128 + 512 + 384 + 384) << 20;
        assert_eq!(reserved(&files, "local", true), (own, 128 << 20));
        // the pointer's host is the habitat's machine under its own name
        assert_eq!(reserved(&files, "box", true), (own, 128 << 20));
    }

    #[test]
    fn another_host_holds_only_the_organisms_applied_there() {
        let files = habitat_files();
        assert_eq!(reserved(&files, "gpu", false), (0, (512 << 20) + (1 << 30)));
        // a host it placed nothing on holds nothing of the habitat's
        assert_eq!(reserved(&files, "spare", false), (0, 0));
        // a habitat whose files are not there holds nothing it can count
        assert_eq!(reserved(&BTreeMap::new(), "local", true), (0, 0));
    }

    #[test]
    fn services_never_applied_count_nowhere() {
        let mut files = habitat_files();
        files.remove("allocations/organism00b/placed.services");
        assert_eq!(reserved(&files, "gpu", false), (0, 512 << 20));
        files.remove("allocations/organism00b/placed.senses");
        assert_eq!(reserved(&files, "local", true).1, (128 + 512) << 20);
    }

    #[test]
    fn the_files_round_trip_through_the_script() {
        let dir = std::env::temp_dir().join(format!("hale-reserved-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("organism");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("compose.yaml"), "services:\n  api:\n    mem_limit: 1g\n").unwrap();
        let mut files = habitat_files();
        files.insert("allocations/organism00b/allocation.json".into(), format!("{{\"root\": \"{}\", \"x\": 1}}", root.display()));
        let expected = files.remove("allocations/organism00b/services.compose.yaml");
        let hab = dir.join("habitat");
        for (p, text) in &files {
            let f = hab.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let out = std::process::Command::new("sh").args(["-c", &files_script(&hab.display().to_string())]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let said = files_said(&String::from_utf8_lossy(&out.stdout)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(expected.is_some());
        assert_eq!(said.get("allocations/organism00b/services.compose.yaml").map(String::as_str), Some("services:\n  api:\n    mem_limit: 1g\n"));
        // allocation.json is read for the root alone, never said
        assert!(!said.contains_key("allocations/organism00b/allocation.json"));
        for (p, text) in files.iter().filter(|(p, _)| !p.ends_with("allocation.json")) {
            assert_eq!(said.get(p), Some(text), "{p}");
        }
        assert_eq!(reserved(&said, "gpu", false), (0, (512 << 20) + (1 << 30)));
    }
}
