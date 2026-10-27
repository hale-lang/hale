//! The habitat's readings of its machines: a Prometheus node exporter on
//! every machine it runs on (its own and each host it adds), scraped by its
//! senses' store, and a Grafana behind its face that charts them.
//!
//! The exporter is the machine's, not an organism's: one per machine on
//! `:9100`, installed over the person's own login (where sudo can ask), and
//! one that already answers there with node readings is scraped as it is and
//! never touched. On Linux it is a container on the host's network and
//! process namespace reading the root filesystem; on macOS the release's
//! own binary under launchd (a LaunchAgent in a person's GUI session, else
//! a LaunchDaemon), since docker there runs in a VM and a container would
//! read the VM.

use std::path::Path;
use std::process::{Command, Stdio};

/// The node exporter release the habitat installs, and its macOS archives'
/// SHA-256 (the release's `sha256sums.txt`).
pub(super) const NODE_EXPORTER_VERSION: &str = "1.12.1";
const NODE_EXPORTER_DARWIN_ARM64_SHA256: &str = "35b246a34af55a8f0c3f11067f17aecf9836cfd4689bde364776b3a4ad3a84d6";
const NODE_EXPORTER_DARWIN_AMD64_SHA256: &str = "3506a6d788b768a3c2cd1f58dc97b9ff3fecc5ac9196623d582982db2aa9d0f6";
/// The port every machine's exporter answers on (the exporter's own default).
pub(super) const NODE_EXPORTER_PORT: u16 = 9100;
/// The name the Linux container goes by and the launchd label on macOS.
const NODE_EXPORTER_NAME: &str = "hale-node-exporter";
const NODE_EXPORTER_LABEL: &str = "lang.hale.node-exporter";

/// The Grafana the habitat runs behind its face.
pub(super) const GRAFANA_IMAGE: &str = "grafana/grafana:13.3.0";
/// Grafana's port: this offset in the habitat's head block (the face, the
/// head, the API child, the stub provider and the commands take 0 to 4).
/// It listens on the face's loopback only and is never published.
pub(super) const GRAFANA_OFFSET: u16 = 5;
/// The path the face serves Grafana under.
pub(super) const GRAFANA_PATH: &str = "/grafana/";
/// The habitat's dashboard of its machines.
const HOSTS_DASHBOARD: &str = include_str!("../../../../dna/habitat/grafana/hosts.json");

/// Whether `HALE_DNA_NODE_EXPORTER=off` keeps this run from installing or
/// removing any machine's exporter (the DNA suite's fixtures; the run a
/// habitat's machine makes inside its image, whose exporter the person's
/// machine installs over SSH instead).
pub(super) fn node_exporter_off() -> bool {
    std::env::var("HALE_DNA_NODE_EXPORTER").is_ok_and(|v| v.trim() == "off")
}

/// Whether `HALE_DNA_GRAFANA=off` leaves Grafana out of the habitat.
pub(super) fn grafana_off() -> bool {
    std::env::var("HALE_DNA_GRAFANA").is_ok_and(|v| v.trim() == "off")
}

/// What a script says it did, on its last line: `hale-node: <outcome>`.
const OUTCOME: &str = "hale-node:";

/// The exporter installed on the machine the script runs on, or found: its
/// last line says which (`ours <how>`, `theirs`, or `skip <why>`). `sudo` is
/// how it asks for root where it must (a LaunchDaemon on a Mac with no GUI
/// session; docker for a login outside its group): `sudo` at a terminal,
/// `sudo -n` elsewhere, which then skips rather than fails.
pub(super) fn install_script(sudo: &str) -> String {
    let v = NODE_EXPORTER_VERSION;
    let p = NODE_EXPORTER_PORT;
    let n = NODE_EXPORTER_NAME;
    let l = NODE_EXPORTER_LABEL;
    let arm = NODE_EXPORTER_DARWIN_ARM64_SHA256;
    let amd = NODE_EXPORTER_DARWIN_AMD64_SHA256;
    format!(
        r#"set -e
S="{sudo}"; v={v}; p={p}; n={n}; l={l}
said() {{ printf '{OUTCOME} %s\n' "$*"; }}
readings() {{ if command -v curl >/dev/null 2>&1; then curl -fsS --max-time 3 "http://127.0.0.1:$p/metrics"; elif command -v wget >/dev/null 2>&1; then wget -qO- -T 3 "http://127.0.0.1:$p/metrics"; fi; }}
node_readings() {{ readings 2>/dev/null | grep -q '^node_'; }}
case "$(uname -s)" in
Linux)
  if docker info >/dev/null 2>&1; then d=docker; elif $S docker info >/dev/null 2>&1; then d="$S docker"; else said "skip docker does not answer this login"; exit 0; fi
  if [ "$($d inspect -f '{{{{.State.Running}}}}' "$n" 2>/dev/null)" = true ]; then said "ours container"; exit 0; fi
  if node_readings; then said theirs; exit 0; fi
  $d rm -f "$n" >/dev/null 2>&1 || true
  $d run -d --name "$n" --label "$l=$v" --restart unless-stopped --net host --pid host -v /:/host:ro,rslave "prom/node-exporter:v$v" --path.rootfs=/host --web.listen-address=":$p" >/dev/null
  said "ours container" ;;
Darwin)
  case "$(uname -m)" in arm64) a=arm64; sum={arm} ;; x86_64) a=amd64; sum={amd} ;; *) said "skip no node_exporter build for $(uname -m)"; exit 0 ;; esac
  if launchctl print "gui/$(id -u)" >/dev/null 2>&1; then
    dom="gui/$(id -u)"; how=agent; R=; bin="$HOME/.local/share/hale/node-exporter/$v/node_exporter"; plist="$HOME/Library/LaunchAgents/$l.plist"; who=
  else
    dom=system; how=daemon; R="$S"; bin="/usr/local/libexec/hale/node-exporter/$v/node_exporter"; plist="/Library/LaunchDaemons/$l.plist"; who="<key>UserName</key><string>nobody</string>"
  fi
  if launchctl print "$dom/$l" >/dev/null 2>&1; then
    if [ -x "$bin" ] && node_readings; then said "ours $how"; exit 0; fi
  elif node_readings; then said theirs; exit 0; fi
  if [ -n "$R" ] && ! $R true 2>/dev/null; then said "skip sudo needs a terminal on this machine (a LaunchDaemon is root's)"; exit 0; fi
  if [ ! -x "$bin" ]; then
    t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
    curl -fsSL -o "$t/ne.tgz" "https://github.com/prometheus/node_exporter/releases/download/v$v/node_exporter-$v.darwin-$a.tar.gz"
    [ "$(shasum -a 256 "$t/ne.tgz" | cut -d' ' -f1)" = "$sum" ] || {{ echo "node_exporter $v: the download is not the release's (checksum)" >&2; exit 1; }}
    tar -xzf "$t/ne.tgz" -C "$t"
    $R mkdir -p "$(dirname "$bin")"
    $R install -m 755 "$t/node_exporter-$v.darwin-$a/node_exporter" "$bin"
  fi
  x=$(mktemp)
  printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' "<plist version=\"1.0\"><dict><key>Label</key><string>$l</string><key>ProgramArguments</key><array><string>$bin</string><string>--web.listen-address=:$p</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/>$who</dict></plist>" > "$x"
  $R mkdir -p "$(dirname "$plist")"
  $R install -m 644 "$x" "$plist"; rm -f "$x"
  $R launchctl bootout "$dom/$l" >/dev/null 2>&1 || true
  $R launchctl bootstrap "$dom" "$plist"
  i=0; while [ $i -lt 20 ] && ! node_readings; do sleep 0.5; i=$((i + 1)); done
  node_readings || {{ echo "node_exporter $v was started under launchd ($dom/$l) and does not answer on :$p" >&2; exit 1; }}
  said "ours $how" ;;
*) said "skip $(uname -s) is neither Linux nor macOS" ;;
esac
"#
    )
}

/// The habitat's exporter removed from the machine the script runs on (an
/// exporter it did not install is never touched); its last line says
/// `removed` or `none`.
pub(super) fn remove_script(sudo: &str) -> String {
    let n = NODE_EXPORTER_NAME;
    let l = NODE_EXPORTER_LABEL;
    format!(
        r#"set -e
S="{sudo}"; n={n}; l={l}; gone=none
case "$(uname -s)" in
Linux)
  if docker info >/dev/null 2>&1; then d=docker; else d="$S docker"; fi
  if [ -n "$($d ps -aq --filter "name=^$n\$" --filter "label=$l" 2>/dev/null)" ]; then $d rm -f "$n" >/dev/null; gone=removed; fi ;;
Darwin)
  if [ -f "$HOME/Library/LaunchAgents/$l.plist" ]; then
    launchctl bootout "gui/$(id -u)/$l" >/dev/null 2>&1 || true
    rm -f "$HOME/Library/LaunchAgents/$l.plist"; rm -rf "$HOME/.local/share/hale/node-exporter"; gone=removed
  fi
  if [ -f "/Library/LaunchDaemons/$l.plist" ]; then
    $S launchctl bootout "system/$l" >/dev/null 2>&1 || true
    $S rm -f "/Library/LaunchDaemons/$l.plist"; $S rm -rf /usr/local/libexec/hale/node-exporter; gone=removed
  fi ;;
esac
printf '{OUTCOME} %s\n' "$gone"
"#
    )
}

/// The outcome a script's output ends with ("" when it said none).
pub(super) fn outcome_of(text: &str) -> String {
    text.lines().rev().find_map(|l| l.trim_end_matches('\r').strip_prefix(OUTCOME)).map(|o| o.trim().to_string()).unwrap_or_default()
}

/// What an install's outcome means, as `up` and `host add` say it of `on`.
pub(super) fn install_line(on: &str, outcome: &str) -> String {
    let v = NODE_EXPORTER_VERSION;
    let p = NODE_EXPORTER_PORT;
    let said = match outcome.split_once(' ').map_or((outcome, ""), |(a, b)| (a, b)) {
        ("ours", "container") => format!("node_exporter {v} on :{p}, a container of the habitat's ({NODE_EXPORTER_NAME})"),
        ("ours", "agent") => format!("node_exporter {v} on :{p}, a launchd agent of yours ({NODE_EXPORTER_LABEL})"),
        ("ours", "daemon") => format!("node_exporter {v} on :{p}, a launchd daemon ({NODE_EXPORTER_LABEL})"),
        ("theirs", _) => format!("an exporter already answers on :{p} with node readings: scraped as it is"),
        ("skip", why) => format!("no exporter: {why}"),
        _ => format!("no exporter: the install said nothing it did ({outcome:?})"),
    };
    format!("{:<8}{on}: {said}", "node")
}

/// A script run on this machine, its output read for the outcome (sudo
/// asks on the terminal itself, never on the output). The outcome, or why
/// the script failed.
pub(super) fn run_here(script: &str) -> Result<String, String> {
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let out = Command::new("sh")
        .args(["-c", script])
        .stdin(if tty { Stdio::inherit() } else { Stdio::null() })
        .stderr(if tty { Stdio::inherit() } else { Stdio::piped() })
        .output()
        .map_err(|e| format!("sh: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        Ok(outcome_of(&text))
    } else {
        Err(last_words(&String::from_utf8_lossy(&out.stderr), &text))
    }
}

/// The last non-empty line of a failed script's error output, else of its
/// output.
pub(super) fn last_words(stderr: &str, stdout: &str) -> String {
    let pick = |t: &str| t.lines().map(|l| l.trim_end_matches('\r').trim()).filter(|l| !l.is_empty() && !l.starts_with(OUTCOME)).last().map(str::to_string);
    pick(stderr).or_else(|| pick(stdout)).unwrap_or_else(|| "it failed and said nothing".to_string())
}

/// The `sudo` a script uses: one that may ask at a terminal, else one that
/// never asks.
pub(super) fn sudo_word(tty: bool) -> &'static str {
    if tty {
        "sudo"
    } else {
        "sudo -n"
    }
}

/// The habitat's own machine, installed here unless the run says not to.
pub(super) fn install_here(label: &str) -> Option<String> {
    if node_exporter_off() {
        return None;
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    super::progress("installing the node exporter on this machine");
    Some(match run_here(&install_script(sudo_word(tty))) {
        Ok(o) => install_line(label, &o),
        Err(why) => format!("{:<8}{label}: no exporter: {why}", "node"),
    })
}

/// The habitat's exporter taken off this machine.
pub(super) fn remove_here() -> Option<String> {
    if node_exporter_off() {
        return None;
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    Some(match run_here(&remove_script(sudo_word(tty))) {
        Ok(o) if o == "removed" => "the habitat's node exporter is removed from this machine".to_string(),
        Ok(_) => String::new(),
        Err(why) => format!("the habitat's node exporter is left on this machine ({why})"),
    })
}

// ---------------------------------------------------------------
// what the senses' store scrapes of the machines
// ---------------------------------------------------------------

/// The scrape targets' file of the habitat's senses' store, in its
/// allocation's `senses.d/`: the habitat's own machine (reached from the
/// store's container over the host gateway) until the head writes the
/// hosts it adds beside it (`dna/api/habitat_hosts.hl`). Written only when
/// there is none.
pub(super) fn seed_targets(allocation: &Path, label: &str) -> Result<(), String> {
    let dir = allocation.join("senses.d");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join("hosts.json");
    if file.exists() {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&serde_json::json!([
        { "targets": [format!("host.docker.internal:{NODE_EXPORTER_PORT}")], "labels": { "host": label } }
    ]))
    .unwrap_or_default();
    std::fs::write(&file, format!("{text}\n")).map_err(|e| format!("write {}: {e}", file.display()))
}

// ---------------------------------------------------------------
// Grafana
// ---------------------------------------------------------------

/// The header Grafana trusts for the person's name: a name only the face
/// and Grafana know (drawn once per habitat, 0600), so nothing else on the
/// machine's loopback can present it. Grafana's proxy authentication has
/// no secret of its own; the header's name is it.
pub(super) fn grafana_header(hab_dir: &Path) -> Result<String, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = hab_dir.join("grafana.secrets");
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim();
        if t.starts_with("X-Hale-Grafana-") && t.len() == "X-Hale-Grafana-".len() + 32 {
            return Ok(t.to_string());
        }
    }
    let mut bytes = [0u8; 16];
    getrandom(&mut bytes)?;
    let name = format!("X-Hale-Grafana-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path).map_err(|e| format!("write {}: {e}", path.display()))?;
    f.write_all(format!("{name}\n").as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(name)
}

fn getrandom(buf: &mut [u8]) -> Result<(), String> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).map_err(|e| format!("/dev/urandom: {e}"))
}

/// Grafana's provisioning under `<habitat>/grafana/provisioning`: the
/// habitat's senses' store as its one data source, and the habitat's
/// dashboards. Written at every `up`, so a toolchain's newer dashboard
/// replaces the last.
pub(super) fn write_provisioning(hab_dir: &Path, prometheus_url: &str) -> Result<(), String> {
    let root = hab_dir.join("grafana").join("provisioning");
    let sources = root.join("datasources");
    let boards = root.join("dashboards");
    for d in [&sources, &boards, &hab_dir.join("grafana").join("data")] {
        std::fs::create_dir_all(d).map_err(|e| format!("create {}: {e}", d.display()))?;
    }
    let ds = format!(
        "apiVersion: 1\ndatasources:\n  - name: habitat senses\n    uid: habitat-senses\n    type: prometheus\n    access: proxy\n    url: {prometheus_url}\n    isDefault: true\n    editable: false\n    jsonData:\n      timeInterval: 15s\n"
    );
    std::fs::write(sources.join("habitat.yaml"), ds).map_err(|e| format!("write {}: {e}", sources.display()))?;
    let provider = "apiVersion: 1\nproviders:\n  - name: habitat\n    folder: habitat\n    type: file\n    disableDeletion: true\n    allowUiUpdates: false\n    options:\n      path: /etc/grafana/provisioning/dashboards\n";
    std::fs::write(boards.join("habitat.yaml"), provider).map_err(|e| format!("write {}: {e}", boards.display()))?;
    std::fs::write(boards.join("hosts.json"), HOSTS_DASHBOARD).map_err(|e| format!("write {}: {e}", boards.display()))?;
    Ok(())
}

/// The habitat's `grafana` compose service: in the face's network namespace
/// (the face reaches it on the loopback, and nothing is published), on
/// `port`, served under `/grafana/` of the face at `face_url`, trusting
/// `header` for the person's name. `user_spec` is the face's own user.
pub(super) fn grafana_service(hab_dir: &str, port: u16, face_url: &str, header: &str, user_spec: &str) -> String {
    // drawn at every `up`: Grafana takes it only when it makes its
    // database, and no one signs in with it (the login form is off)
    let mut bytes = [0u8; 16];
    let admin_password = if getrandom(&mut bytes).is_ok() { bytes.iter().map(|b| format!("{b:02x}")).collect::<String>() } else { format!("{:x}", std::process::id() as u64 * 0x9e37_79b9) };
    let mut s = String::from("  grafana:\n");
    s.push_str(&format!("    image: \"{GRAFANA_IMAGE}\"\n"));
    s.push_str("    network_mode: \"service:face\"\n");
    s.push_str(user_spec);
    s.push_str("    depends_on:\n      - \"face\"\n");
    s.push_str("    environment:\n");
    for (k, v) in [
        ("GF_SERVER_HTTP_ADDR", "127.0.0.1".to_string()),
        ("GF_SERVER_HTTP_PORT", port.to_string()),
        ("GF_SERVER_ROOT_URL", format!("{}{GRAFANA_PATH}", face_url.trim_end_matches('/'))),
        ("GF_SERVER_SERVE_FROM_SUB_PATH", "true".to_string()),
        ("GF_AUTH_PROXY_ENABLED", "true".to_string()),
        ("GF_AUTH_PROXY_HEADER_NAME", header.to_string()),
        ("GF_AUTH_PROXY_HEADER_PROPERTY", "username".to_string()),
        ("GF_AUTH_PROXY_AUTO_SIGN_UP", "true".to_string()),
        ("GF_AUTH_PROXY_WHITELIST", "127.0.0.1, ::1".to_string()),
        ("GF_AUTH_DISABLE_LOGIN_FORM", "true".to_string()),
        ("GF_AUTH_SIGNOUT_REDIRECT_URL", "/auth/logout".to_string()),
        ("GF_AUTH_ANONYMOUS_ENABLED", "false".to_string()),
        // no credential of Grafana's own opens it: not its basic
        // authentication, and not an admin password anyone could know
        ("GF_AUTH_BASIC_ENABLED", "false".to_string()),
        ("GF_SECURITY_ADMIN_PASSWORD", admin_password.to_string()),
        ("GF_USERS_ALLOW_SIGN_UP", "false".to_string()),
        ("GF_USERS_AUTO_ASSIGN_ORG_ROLE", "Editor".to_string()),
        ("GF_LIVE_MAX_CONNECTIONS", "0".to_string()),
        // it runs as the habitat's user, who owns no file under the image's
        // log directory: its log is compose's
        ("GF_LOG_MODE", "console".to_string()),
        ("GF_ANALYTICS_REPORTING_ENABLED", "false".to_string()),
        ("GF_ANALYTICS_CHECK_FOR_UPDATES", "false".to_string()),
        ("GF_ANALYTICS_CHECK_FOR_PLUGIN_UPDATES", "false".to_string()),
        ("GF_NEWS_NEWS_FEED_ENABLED", "false".to_string()),
        ("GF_DASHBOARDS_DEFAULT_HOME_DASHBOARD_PATH", "/etc/grafana/provisioning/dashboards/hosts.json".to_string()),
    ] {
        s.push_str(&format!("      {k}: \"{v}\"\n"));
    }
    s.push_str(&format!(
        "    volumes:\n      - \"{hab_dir}/grafana/provisioning:/etc/grafana/provisioning:ro\"\n      - \"{hab_dir}/grafana/data:/var/lib/grafana\"\n    restart: on-failure\n"
    ));
    s
}

/// The habitat senses' store's port on this machine's loopback, from its
/// allocation's `ports.yaml` (`senses-<key>: ports: ["127.0.0.1:<port>:9090"]`).
pub(super) fn senses_port(allocation: &Path, key: &str) -> Option<u16> {
    let text = std::fs::read_to_string(allocation.join("ports.yaml")).ok()?;
    let at = text.find(&format!("\n  senses-{key}:\n"))?;
    let rest = &text[at..];
    let line = rest.lines().find(|l| l.contains(":9090\""))?;
    let inner = line.split('"').nth(1)?;
    inner.rsplit(':').nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_pins_the_release_and_never_asks_for_sudo_away_from_a_terminal() {
        let s = install_script("sudo -n");
        assert!(s.contains("S=\"sudo -n\""));
        assert!(s.contains(&format!("v={NODE_EXPORTER_VERSION}")));
        assert!(s.contains(NODE_EXPORTER_DARWIN_ARM64_SHA256) && s.contains(NODE_EXPORTER_DARWIN_AMD64_SHA256));
        assert!(s.contains("prom/node-exporter:v$v"));
        // a Mac's daemon runs as nobody; the format's braces are the shell's
        assert!(s.contains("<key>UserName</key><string>nobody</string>"));
        assert!(s.contains("'{{.State.Running}}'"), "docker's template survives the format");
    }

    #[test]
    fn the_removal_touches_only_the_habitats_own() {
        let s = remove_script("sudo");
        assert!(s.contains("--filter \"label=$l\""), "a container is the habitat's by its label");
        assert!(s.contains("/Library/LaunchDaemons/$l.plist"));
    }

    #[test]
    fn an_outcome_is_the_last_line_that_says_one() {
        assert_eq!(outcome_of("noise\nhale-node: ours daemon\r\n"), "ours daemon");
        assert_eq!(outcome_of("nothing"), "");
        assert!(install_line("box", "theirs").contains("scraped as it is"));
        assert!(install_line("box", "skip sudo needs a terminal").ends_with("no exporter: sudo needs a terminal"));
        assert_eq!(last_words("\nfirst\nlast\n", ""), "last");
    }

    #[test]
    fn grafana_trusts_the_header_from_the_loopback_only_and_is_never_published() {
        let s = grafana_service("/h", 8805, "http://10.0.0.5:8800/", "X-Hale-Grafana-00", "");
        assert!(s.contains("network_mode: \"service:face\""));
        assert!(s.contains("GF_SERVER_HTTP_ADDR: \"127.0.0.1\""));
        assert!(s.contains("GF_SERVER_ROOT_URL: \"http://10.0.0.5:8800/grafana/\""));
        assert!(s.contains("GF_AUTH_PROXY_HEADER_NAME: \"X-Hale-Grafana-00\""));
        assert!(!s.contains("ports:"));
        assert!(s.contains("GF_AUTH_BASIC_ENABLED: \"false\""));
        assert!(!s.contains("GF_SECURITY_ADMIN_PASSWORD: \"admin\""));
    }

    #[test]
    fn the_senses_port_is_read_from_the_allocations_ports() {
        let d = std::env::temp_dir().join(format!("hale-readings-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("ports.yaml"), "services:\n  memory-k:\n    ports: [\"127.0.0.1:17001:5432\"]\n  senses-k:\n    ports: [\"127.0.0.1:19009:9090\"]\n").unwrap();
        assert_eq!(senses_port(&d, "k"), Some(19009));
        assert_eq!(senses_port(&d, "other"), None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_hosts_dashboard_is_json() {
        let v: serde_json::Value = serde_json::from_str(HOSTS_DASHBOARD).expect("hosts.json parses");
        assert_eq!(v["uid"], "hale-hosts");
    }
}
