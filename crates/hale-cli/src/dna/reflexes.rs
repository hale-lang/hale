//! Reflexes: what an organism watches for in its senses, declared as data.
//!
//! A reflex is one model whatever declared it and wherever it runs: a
//! condition over the organism's readings (PromQL), how long it must hold,
//! how long it keeps firing once it stops holding, a severity, the action it
//! names, and a summary. The manifest's `[dna.reflex.<name>]` is its front
//! end today. Its evaluator is a Prometheus store: the model renders to a
//! standard rule group, which the habitat's senses' store reads beside its
//! scrape configuration (and which a heart run by hand can hand its own
//! Prometheus). Nothing acts on a reflex yet: its alerts are read
//! (`Commands::alerts`, the operator face's alerts screen), and its
//! `action` is carried on the alert for the receiver that will act.
//!
//! ```text
//! [dna.reflex.disk-filling]
//! when     = '100 * (1 - node_filesystem_avail_bytes / node_filesystem_size_bytes) > 90'
//! for      = "10m"
//! keep     = "10m"           # keeps firing this long after it stops holding
//! severity = "warning"       # or "critical"
//! action   = "concern"       # concern | restart | notify
//! summary  = "{host}:{mountpoint} is {value}% full"
//! ```

use std::collections::BTreeMap;
use std::path::Path;

/// One reflex, as every front end and evaluator knows it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Reflex {
    pub name: String,
    pub when: String,
    pub hold: String,
    pub keep: String,
    pub severity: String,
    pub action: String,
    pub summary: String,
}

const ACTIONS: &[&str] = &["concern", "restart", "notify"];
const SEVERITIES: &[&str] = &["warning", "critical"];

/// Whether `s` is a Prometheus duration this model takes (`<n>s|m|h|d`).
fn duration(s: &str) -> bool {
    let (n, unit) = s.split_at(s.len().saturating_sub(1));
    !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && ["s", "m", "h", "d"].contains(&unit)
}

/// The reflexes a manifest declares (`[dna.reflex.<name>]`), checked.
pub(super) fn declared(manifest: &str) -> Result<Vec<Reflex>, String> {
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| format!("hale.toml: {e}"))?;
    let Some(t) = doc.get("dna").and_then(|d| d.get("reflex")) else { return Ok(Vec::new()) };
    let t = t.as_table().ok_or("[dna.reflex] is a table of reflexes")?;
    let mut out = Vec::new();
    for (name, r) in t {
        let r = r.as_table().ok_or_else(|| format!("[dna.reflex.{name}] is a table"))?;
        for k in r.keys() {
            if !["when", "for", "keep", "severity", "action", "summary"].contains(&k.as_str()) {
                return Err(format!("[dna.reflex.{name}] has no `{k}`"));
            }
        }
        let s = |k: &str, default: &str| -> Result<String, String> {
            match r.get(k) {
                None => Ok(default.to_string()),
                Some(v) => v.as_str().map(str::to_string).ok_or_else(|| format!("[dna.reflex.{name}] {k} is a string")),
            }
        };
        out.push(checked(Reflex {
            name: name.clone(),
            when: s("when", "")?,
            hold: s("for", "5m")?,
            keep: s("keep", "")?,
            severity: s("severity", "warning")?,
            action: s("action", "concern")?,
            summary: s("summary", "")?,
        })?);
    }
    Ok(out)
}

fn checked(r: Reflex) -> Result<Reflex, String> {
    let at = format!("[dna.reflex.{}]", r.name);
    if r.name.is_empty() || !r.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("{at}: a reflex is named by letters, digits, `-` and `_`"));
    }
    if r.when.trim().is_empty() {
        return Err(format!("{at} names no condition (`when`)"));
    }
    if !duration(&r.hold) || (!r.keep.is_empty() && !duration(&r.keep)) {
        return Err(format!("{at}: `for` and `keep` are durations (`10m`, `1h`)"));
    }
    if !SEVERITIES.contains(&r.severity.as_str()) {
        return Err(format!("{at}: severity is {}", SEVERITIES.join(" or ")));
    }
    if !ACTIONS.contains(&r.action.as_str()) {
        return Err(format!("{at}: action is one of {}", ACTIONS.join(", ")));
    }
    Ok(r)
}

/// The real filesystems of the machines' exporters: what a disk reflex
/// watches (the Hosts dashboard's own filter).
const REAL_FS: &str = r#"job="host", fstype!~"tmpfs|devtmpfs|overlay|squashfs|ramfs|autofs|nullfs|fuse.*|nfs.*", mountpoint!~"/System/Volumes/(VM|Preboot|Update|xarts|iSCPreboot|Hardware)|/boot.*|/snap/.*|/run.*|/var/lib/docker/.*|/dev.*""#;

/// A machine's memory in use, in percent, as the Hosts dashboard reads it:
/// all but what Linux says is available, or a Mac's active, wired and
/// compressed pages (node_exporter's Darwin collector has no `available`).
const MEMORY_USED: &str = r#"max by (host) (100 * (1 - node_memory_MemAvailable_bytes{job="host"} / node_memory_MemTotal_bytes{job="host"})) or max by (host) (100 * (node_memory_active_bytes{job="host"} + node_memory_wired_bytes{job="host"} + node_memory_compressed_bytes{job="host"}) / node_memory_total_bytes{job="host"})"#;

/// The habitat's own reflexes, which every habitat watches for on any
/// machine it uses: a disk nearly full, or filling within a day; its memory
/// nearly all in use; its CPU nearly all busy, for long enough that it is
/// no burst.
pub(super) fn habitat_defaults() -> Vec<Reflex> {
    vec![
        Reflex {
            name: "memory-pressure".into(),
            when: format!("({MEMORY_USED}) > 90"),
            hold: "15m".into(),
            keep: "10m".into(),
            severity: "warning".into(),
            action: "concern".into(),
            summary: "{host}'s memory is {value}% in use".into(),
        },
        Reflex {
            name: "cpu-saturated".into(),
            when: r#"100 * (1 - avg by (host) (rate(node_cpu_seconds_total{job="host", mode="idle"}[5m]))) > 90"#.into(),
            hold: "15m".into(),
            keep: "10m".into(),
            severity: "warning".into(),
            action: "concern".into(),
            summary: "{host}'s CPU is {value}% busy".into(),
        },
        Reflex {
            name: "disk-filling".into(),
            when: format!("100 * (1 - node_filesystem_avail_bytes{{{REAL_FS}}} / node_filesystem_size_bytes{{{REAL_FS}}}) > 90"),
            hold: "10m".into(),
            keep: "10m".into(),
            severity: "warning".into(),
            action: "concern".into(),
            summary: "{host}:{mountpoint} is {value}% full".into(),
        },
        Reflex {
            name: "disk-full-within-a-day".into(),
            when: format!("predict_linear(node_filesystem_avail_bytes{{{REAL_FS}}}[6h], 86400) < 0"),
            hold: "1h".into(),
            keep: "30m".into(),
            severity: "warning".into(),
            action: "concern".into(),
            summary: "{host}:{mountpoint} fills within a day at its last six hours' rate".into(),
        },
    ]
}

/// A summary's `{label}` and `{value}` as Prometheus's templates say them.
fn template(summary: &str) -> String {
    let mut out = String::new();
    let mut rest = summary;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else { break };
        let word = &rest[i + 1..i + j];
        if word == "value" {
            out.push_str("{{ $value | printf \"%.0f\" }}");
        } else if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push_str(&format!("{{{{ $labels.{word} }}}}"));
        } else {
            out.push_str(&rest[i..=i + j]);
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

/// A YAML string, double-quoted.
fn yq(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The reflexes of the organism keyed `key` as one Prometheus rule group:
/// each an alert named for the reflex, labelled with the reflex, its
/// organism, severity and action.
pub(super) fn render(key: &str, reflexes: &[Reflex]) -> String {
    let mut s = String::from("# The organism's reflexes, rendered from its declarations and the habitat's\n# defaults by `hale habitat up`; rewritten whole at each.\ngroups:\n");
    s.push_str(&format!("  - name: {}\n    rules:\n", yq(&format!("reflexes-{key}"))));
    for r in reflexes {
        s.push_str(&format!("      - alert: {}\n        expr: {}\n        for: {}\n", yq(&r.name), yq(&r.when), r.hold));
        if !r.keep.is_empty() {
            s.push_str(&format!("        keep_firing_for: {}\n", r.keep));
        }
        s.push_str(&format!(
            "        labels:\n          reflex: {}\n          organism: {}\n          severity: {}\n          action: {}\n",
            yq(&r.name),
            yq(key),
            yq(&r.severity),
            yq(&r.action)
        ));
        if !r.summary.is_empty() {
            s.push_str(&format!("        annotations:\n          summary: {}\n", yq(&template(&r.summary))));
        }
    }
    s
}

/// The rules' file under the store's `senses.d/`, and the line its
/// configuration reads it by.
const RULES_FILE: &str = "reflexes.rules.yml";
const RULE_FILES_LINE: &str = "rule_files:\n  - /etc/prometheus/senses.d/*.rules.yml\n";

/// The habitat's reflexes written for its store, in its allocation `dir`
/// (keyed `key`): its defaults and what its genome declares. The store's
/// configuration reads the rules' file from then on. Whether either file
/// changed, so the caller restarts the store to read them.
pub(super) fn write_habitat_rules(dir: &Path, key: &str, genome_manifest: &str) -> Result<bool, String> {
    let mut reflexes: BTreeMap<String, Reflex> = habitat_defaults().into_iter().map(|r| (r.name.clone(), r)).collect();
    // the genome's own replace a default of the same name
    for r in declared(genome_manifest)? {
        reflexes.insert(r.name.clone(), r);
    }
    let rules = render(key, &reflexes.into_values().collect::<Vec<_>>());
    let d = dir.join("senses.d");
    std::fs::create_dir_all(&d).map_err(|e| format!("create {}: {e}", d.display()))?;
    let path = d.join(RULES_FILE);
    let mut changed = std::fs::read_to_string(&path).ok().as_deref() != Some(rules.as_str());
    if changed {
        std::fs::write(&path, &rules).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    let conf = dir.join("senses.yml");
    if let Ok(text) = std::fs::read_to_string(&conf) {
        if !text.contains("rule_files:") {
            std::fs::write(&conf, format!("{}\n# The organism's reflexes (`[dna.reflex.*]`, and a habitat's defaults).\n{RULE_FILES_LINE}", text.trim_end())).map_err(|e| format!("write {}: {e}", conf.display()))?;
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reflex_is_read_and_checked() {
        let m = "[dna.reflex.queue-deep]\nwhen = 'voice_queue_depth > 100'\nfor = \"5m\"\nseverity = \"critical\"\naction = \"notify\"\nsummary = \"{instance} has {value} waiting\"\n";
        let r = &declared(m).unwrap()[0];
        assert_eq!((r.name.as_str(), r.hold.as_str(), r.severity.as_str(), r.action.as_str()), ("queue-deep", "5m", "critical", "notify"));
        assert!(declared("[dna]\nneeds = []\n").unwrap().is_empty());
        for (bad, says) in [("when = 'x'\nfor = \"soon\"", "durations"), ("when = 'x'\naction = \"reboot\"", "action is one of"), ("when = 'x'\ncolor = \"red\"", "has no `color`"), ("for = \"5m\"", "names no condition")] {
            let e = declared(&format!("[dna.reflex.r]\n{bad}\n")).unwrap_err();
            assert!(e.contains(says), "{bad}: {e}");
        }
    }

    #[test]
    fn a_reflex_renders_as_a_prometheus_alert() {
        let rules = render("abc123", &habitat_defaults());
        assert!(rules.contains("- alert: \"disk-filling\""), "{rules}");
        assert!(rules.contains("for: 10m\n        keep_firing_for: 10m"), "{rules}");
        assert!(rules.contains("organism: \"abc123\"") && rules.contains("action: \"concern\""), "{rules}");
        assert!(rules.contains(r#"summary: "{{ $labels.host }}:{{ $labels.mountpoint }} is {{ $value | printf \"%.0f\" }}% full""#), "{rules}");
        // the PromQL survives as one YAML string: its quotes escaped
        assert!(rules.contains(r#"fstype!~\"tmpfs"#), "{rules}");
        // memory and CPU, on Linux's and a Mac's exporter alike
        assert!(rules.contains("- alert: \"memory-pressure\"") && rules.contains("node_memory_MemAvailable_bytes") && rules.contains("node_memory_compressed_bytes"), "{rules}");
        assert!(rules.contains("- alert: \"cpu-saturated\"") && rules.contains(r#"mode=\"idle\""#) && rules.contains("for: 15m"), "{rules}");
        assert!(rules.contains(r#"summary: "{{ $labels.host }}'s CPU is {{ $value | printf \"%.0f\" }}% busy""#), "{rules}");
    }

    #[test]
    fn the_habitat_writes_its_rules_and_its_store_reads_them_once() {
        let d = std::env::temp_dir().join(format!("hale-reflexes-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("senses.yml"), "global:\n  scrape_interval: 5s\n").unwrap();
        let genome = "[dna.reflex.disk-filling]\nwhen = 'up == 0'\nfor = \"1m\"\n";
        assert!(write_habitat_rules(&d, "k", genome).unwrap(), "first: written");
        let rules = std::fs::read_to_string(d.join("senses.d").join(RULES_FILE)).unwrap();
        assert!(rules.contains("expr: \"up == 0\"") && rules.contains("disk-full-within-a-day"), "the genome's replaces the default of its name: {rules}");
        assert_eq!(std::fs::read_to_string(d.join("senses.yml")).unwrap().matches("rule_files:").count(), 1);
        assert!(!write_habitat_rules(&d, "k", genome).unwrap(), "again: nothing changed");
        std::fs::remove_dir_all(&d).ok();
    }
}
