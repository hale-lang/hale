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
//!
//! A Linux machine with an NVIDIA GPU runs a GPU exporter beside them:
//! `nvidia_gpu_exporter`, a container given the GPU by docker's NVIDIA
//! runtime, reading `nvidia-smi`.

use std::path::Path;
use std::process::{Command, Stdio};

/// The node exporter release the habitat installs, and its macOS archives'
/// SHA-256 (the release's `sha256sums.txt`). 1.11.1, not 1.12: 1.12's
/// cpufreq collector reads `cpuinfo_avg_freq`, which a 7.0 kernel answers
/// with EAGAIN for a core that has been idle, and Go's read of a sysfs file
/// then waits on the poller forever, so every scrape of such a machine
/// hangs. 1.11 reads the frequencies (`scaling_cur_freq`, as htop does)
/// and nothing else.
pub(super) const NODE_EXPORTER_VERSION: &str = "1.11.1";
const NODE_EXPORTER_DARWIN_ARM64_SHA256: &str = "e987428618362c2d2540a68b722bd982ef1486c9961631298f20ea8fd57d3be4";
const NODE_EXPORTER_DARWIN_AMD64_SHA256: &str = "782318ceb48cb5501271a666d1b015a9406c02cd45dbc9513deca005b91e03a5";
/// The port every machine's exporter answers on (the exporter's own default).
pub(super) const NODE_EXPORTER_PORT: u16 = 9100;
/// The name the Linux container goes by and the launchd label on macOS.
const NODE_EXPORTER_NAME: &str = "hale-node-exporter";
/// What the Linux container runs with beyond its address (none today).
/// Its container is labelled with its version and these, so an install
/// replaces one of the habitat's made otherwise.
const NODE_EXPORTER_LINUX_FLAGS: &str = "";
const NODE_EXPORTER_LABEL: &str = "lang.hale.node-exporter";

/// The container exporter every machine runs beside its node exporter:
/// cAdvisor, as a container wherever docker runs (on a Mac, inside docker's
/// VM, where the containers are), on its own port, reading docker's
/// containers only, keeping their compose project and service and nothing
/// else of their labels, and collecting CPU, memory, network and OOM
/// events every 15 s (its heavier collectors off). docker's socket is
/// mounted by its own path besides `/var/run`: inside a desktop VM it is a
/// symlink to the person's machine, which no container resolves.
pub(super) const CADVISOR_IMAGE: &str = "ghcr.io/google/cadvisor:v0.60.6";
pub(super) const CADVISOR_PORT: u16 = 9338;
const CADVISOR_NAME: &str = "hale-cadvisor";
const CADVISOR_LABEL: &str = "lang.hale.cadvisor";
const CONTAINERS_OUTCOME: &str = "hale-containers:";

/// The GPU exporter a Linux machine with an NVIDIA GPU runs beside its node
/// exporter: nvidia_gpu_exporter 1.17.0, the image's plain (`exec`) flavor,
/// which reads `nvidia-smi` (the NVIDIA runtime puts the host's into the
/// container), by its tag and the digest of the release's image index
/// (Docker Hub's and GHCR's agree), so docker refuses any other content.
/// On the exporter's own port. A machine with no such GPU (every Mac) runs
/// none.
pub(super) const GPU_EXPORTER_VERSION: &str = "1.17.0";
pub(super) const GPU_EXPORTER_IMAGE: &str = "utkuozdemir/nvidia_gpu_exporter:1.17.0@sha256:b085013de2828d7045c80f6af3db030b69fd81c14a43384c77652f74ad87971c";
pub(super) const GPU_EXPORTER_PORT: u16 = 9835;
const GPU_EXPORTER_NAME: &str = "hale-gpu-exporter";
const GPU_EXPORTER_LABEL: &str = "lang.hale.gpu-exporter";
const GPUS_OUTCOME: &str = "hale-gpus:";

/// The Grafana the habitat runs behind its face.
pub(super) const GRAFANA_IMAGE: &str = "grafana/grafana:13.3.0";
/// Grafana's port: this offset in the habitat's head block (the face, the
/// head, the API child, the stub provider and the commands take 0 to 4).
/// It listens on the face's loopback only and is never published.
pub(super) const GRAFANA_OFFSET: u16 = 5;
/// The path the face serves Grafana under.
pub(super) const GRAFANA_PATH: &str = "/grafana/";
/// The habitat's dashboards: its machines, and what runs on them (the
/// Services dashboard: its containers and the services it placed on its
/// hosts, one table whatever a host's system; its uid is the Containers
/// dashboard's it replaced, so a link to that one reaches it).
const HOSTS_DASHBOARD: &str = include_str!("../../../../dna/habitat/grafana/hosts.json");
const SERVICES_DASHBOARD: &str = include_str!("../../../../dna/habitat/grafana/services.json");

/// Whether `HALE_DNA_NODE_EXPORTER=off` keeps this run from installing or
/// removing any machine's exporter (the DNA suite's fixtures; the run a
/// habitat's machine makes inside its image, whose exporter the person's
/// machine installs over SSH instead).
pub(super) fn node_exporter_off() -> bool {
    std::env::var("HALE_DNA_NODE_EXPORTER").is_ok_and(|v| v.trim() == "off")
}

/// When the habitat's Grafana runs, as `HALE_DNA_GRAFANA` asks: `off`
/// never, `on` from `up` to `down`, and otherwise on demand: `up` creates
/// its service without starting it, and the face starts it at its first
/// use and stops it once it has gone unused for a while
/// (`dna/face/standard`, `GrafanaKeeper`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GrafanaRuns {
    Never,
    Always,
    OnDemand,
}

/// How long an on-demand Grafana goes unused before the face stops it (the
/// face's `GRAFANA_IDLE_S`).
pub(super) const GRAFANA_IDLE_MINUTES: u64 = 15;

pub(super) fn grafana_runs() -> GrafanaRuns {
    grafana_runs_for(std::env::var("HALE_DNA_GRAFANA").ok().as_deref())
}

fn grafana_runs_for(asked: Option<&str>) -> GrafanaRuns {
    match asked.map(str::trim) {
        Some("off") => GrafanaRuns::Never,
        Some("on") => GrafanaRuns::Always,
        _ => GrafanaRuns::OnDemand,
    }
}

/// Whether `HALE_DNA_GRAFANA=off` leaves Grafana out of the habitat.
pub(super) fn grafana_off() -> bool {
    grafana_runs() == GrafanaRuns::Never
}

/// Grafana's guardrails: a soft limit its Go runtime collects under, and a
/// hard one its container is killed at. Measured on 13.3.0 with the
/// habitat's provisioning and its plugins trimmed (below): about 170 MiB
/// with both dashboards' queries run; 640 MiB with Grafana's defaults.
pub(super) const GRAFANA_GOMEMLIMIT: &str = "200MiB";
pub(super) const GRAFANA_MEM_LIMIT: &str = "384m";
/// The plugins Grafana ships that the habitat never uses: each data
/// source with a backend is a process of its own (15 to 30 MiB apiece),
/// and the apps load at every start. Its one data source is Prometheus.
const GRAFANA_UNUSED_PLUGINS: &str = "cloudwatch,elasticsearch,grafana-postgresql-datasource,grafana-pyroscope-datasource,influxdb,jaeger,loki,mssql,mysql,opentsdb,stackdriver,tempo,zipkin,grafana-advisor-app,grafana-exploretraces-app,grafana-lokiexplore-app,grafana-metricsdrilldown-app,grafana-pyroscope-app";

/// The container labels cAdvisor carries onto every reading: compose's
/// project and service, and the habitat's own (`dna::compose_labels`:
/// which habitat, which organism, which part), which the Containers
/// dashboard groups and filters by.
const CONTAINER_LABELS: &str = "com.docker.compose.project,com.docker.compose.service,hale.habitat,hale.organism,hale.role";

/// `docker run`'s labels for a container of the habitat's own, a part
/// `role` of the habitat (as `dna::compose_labels` writes a service's).
fn docker_labels(role: &str) -> String {
    let q = |v: &str| format!("'{}'", v.replace('\'', "'\\''"));
    format!("--label {} --label hale.organism=habitat --label {}", q(&format!("hale.habitat={}", super::habitat_label())), q(&format!("hale.role={role}")))
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
    let flags = NODE_EXPORTER_LINUX_FLAGS;
    // the container's label: its version, its flags when it has any, and
    // the habitat whose it is (so one labelled for another is replaced)
    let hab = super::habitat_label();
    let want = if flags.is_empty() { format!("{v} {hab}") } else { format!("{v} {flags} {hab}") };
    let labels = docker_labels("exporter");
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
  want="{want}"
  if [ "$($d inspect -f '{{{{.State.Running}}}} {{{{index .Config.Labels "'$l'"}}}}' "$n" 2>/dev/null)" = "true $want" ]; then said "ours container"; exit 0; fi
  if ! $d inspect "$n" >/dev/null 2>&1 && node_readings; then said theirs; exit 0; fi
  $d rm -f "$n" >/dev/null 2>&1 || true
  $d run -d --name "$n" --label "$l=$want" {labels} --restart unless-stopped --net host --pid host -v /:/host:ro,rslave "prom/node-exporter:v$v" --path.rootfs=/host --web.listen-address=":$p" {flags} >/dev/null
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
    text.lines().rev().find_map(|l| { let l = l.trim_end_matches('\r'); l.strip_prefix(OUTCOME).or_else(|| l.strip_prefix(CONTAINERS_OUTCOME)).or_else(|| l.strip_prefix(GPUS_OUTCOME)) }).map(|o| o.trim().to_string()).unwrap_or_default()
}

/// The container exporter installed where the script runs, through
/// whatever docker the login reaches (its own access, else `sudo`): its
/// last line says `ours`, `theirs` (something else already answers on its
/// port with container readings) or `skip <why>`. The container is
/// labelled with its image, so an install replaces one made otherwise.
pub(super) fn containers_script(sudo: &str) -> String {
    let n = CADVISOR_NAME;
    let l = CADVISOR_LABEL;
    let img = CADVISOR_IMAGE;
    let p = CADVISOR_PORT;
    // its label: the image, the labels it carries onto every reading, and the
    // habitat whose it is; one that differs in any is replaced
    let want = format!("{img} {CONTAINER_LABELS} {}", super::habitat_label());
    let labels = docker_labels("exporter");
    format!(
        r#"set -e
S="{sudo}"; n={n}; l={l}; img={img}; p={p}; want="{want}"
said() {{ printf '{CONTAINERS_OUTCOME} %s\n' "$*"; }}
readings() {{ if command -v curl >/dev/null 2>&1; then curl -fsS --max-time 5 "http://127.0.0.1:$p/metrics"; elif command -v wget >/dev/null 2>&1; then wget -qO- -T 5 "http://127.0.0.1:$p/metrics"; fi; }}
if docker info >/dev/null 2>&1; then d=docker; elif $S docker info >/dev/null 2>&1; then d="$S docker"; else said "skip docker does not answer this login"; exit 0; fi
if [ "$($d inspect -f '{{{{.State.Running}}}} {{{{index .Config.Labels "'$l'"}}}}' "$n" 2>/dev/null)" = "true $want" ]; then said ours; exit 0; fi
if ! $d inspect "$n" >/dev/null 2>&1 && readings 2>/dev/null | grep -q '^container_'; then said theirs; exit 0; fi
$d rm -f "$n" >/dev/null 2>&1 || true
$d run -d --name "$n" --label "$l=$want" {labels} --restart unless-stopped --privileged --net host \
  -v /:/rootfs:ro -v /var/run:/var/run:ro -v /var/run/docker.sock:/var/run/docker.sock:ro -v /sys:/sys:ro -v /var/lib/docker/:/var/lib/docker:ro --device /dev/kmsg \
  "$img" -port="$p" -docker_only=true -housekeeping_interval=15s -store_container_labels=false \
  -whitelisted_container_labels={CONTAINER_LABELS} \
  -enable_metrics=cpu,memory,network,oom_event >/dev/null
said ours
"#
    )
}

/// The habitat's container exporter removed where the script runs (one it
/// did not start is never touched): `removed` or `none`.
pub(super) fn containers_remove_script(sudo: &str) -> String {
    let n = CADVISOR_NAME;
    let l = CADVISOR_LABEL;
    format!(
        r#"S="{sudo}"; n={n}; l={l}
if docker info >/dev/null 2>&1; then d=docker; else d="$S docker"; fi
if [ -n "$($d ps -aq --filter "name=^$n\$" --filter "label=$l" 2>/dev/null)" ]; then $d rm -f "$n" >/dev/null && printf '{CONTAINERS_OUTCOME} removed\n'; else printf '{CONTAINERS_OUTCOME} none\n'; fi
"#
    )
}

/// The GPU exporter installed where the script runs, on Linux with an
/// NVIDIA GPU (`nvidia-smi -L` lists one), through whatever docker the login
/// reaches: its last line says `ours`, `theirs` (something else already
/// answers on its port with GPU readings), `skip <why>` (a GPU docker gives
/// no container: no NVIDIA Container Toolkit) or `none <why>` (no such GPU,
/// or not Linux). The container is labelled with its image and the habitat,
/// so an install replaces one made otherwise.
pub(super) fn gpus_script(sudo: &str) -> String {
    let n = GPU_EXPORTER_NAME;
    let l = GPU_EXPORTER_LABEL;
    let img = GPU_EXPORTER_IMAGE;
    let p = GPU_EXPORTER_PORT;
    let want = format!("{img} {}", super::habitat_label());
    let labels = docker_labels("exporter");
    format!(
        r#"set -e
S="{sudo}"; n={n}; l={l}; img={img}; p={p}; want="{want}"
said() {{ printf '{GPUS_OUTCOME} %s\n' "$*"; }}
readings() {{ if command -v curl >/dev/null 2>&1; then curl -fsS --max-time 5 "http://127.0.0.1:$p/metrics"; elif command -v wget >/dev/null 2>&1; then wget -qO- -T 5 "http://127.0.0.1:$p/metrics"; fi; }}
[ "$(uname -s)" = Linux ] || {{ said "none $(uname -s)"; exit 0; }}
if ! command -v nvidia-smi >/dev/null 2>&1 || ! nvidia-smi -L 2>/dev/null | grep -q '^GPU '; then said "none no NVIDIA GPU"; exit 0; fi
if docker info >/dev/null 2>&1; then d=docker; elif $S docker info >/dev/null 2>&1; then d="$S docker"; else said "skip docker does not answer this login"; exit 0; fi
if [ "$($d inspect -f '{{{{.State.Running}}}} {{{{index .Config.Labels "'$l'"}}}}' "$n" 2>/dev/null)" = "true $want" ]; then said ours; exit 0; fi
if ! $d inspect "$n" >/dev/null 2>&1 && readings 2>/dev/null | grep -q '^nvidia_smi_'; then said theirs; exit 0; fi
$d rm -f "$n" >/dev/null 2>&1 || true
if ! why=$($d run -d --name "$n" --label "$l=$want" {labels} --restart unless-stopped --net host \
  --gpus all -e NVIDIA_DRIVER_CAPABILITIES=utility "$img" --web.listen-address=":$p" 2>&1 >/dev/null); then
  $d rm -f "$n" >/dev/null 2>&1 || true
  said "skip docker gives no container the GPU (the NVIDIA Container Toolkit): $(printf '%s\n' "$why" | tail -n 1)"; exit 0
fi
said ours
"#
    )
}

/// The habitat's GPU exporter removed where the script runs (one it did not
/// start is never touched): `removed` or `none`.
pub(super) fn gpus_remove_script(sudo: &str) -> String {
    let n = GPU_EXPORTER_NAME;
    let l = GPU_EXPORTER_LABEL;
    format!(
        r#"S="{sudo}"; n={n}; l={l}
[ "$(uname -s)" = Linux ] || {{ printf '{GPUS_OUTCOME} none\n'; exit 0; }}
if docker info >/dev/null 2>&1; then d=docker; else d="$S docker"; fi
if [ -n "$($d ps -aq --filter "name=^$n\$" --filter "label=$l" 2>/dev/null)" ]; then $d rm -f "$n" >/dev/null && printf '{GPUS_OUTCOME} removed\n'; else printf '{GPUS_OUTCOME} none\n'; fi
"#
    )
}

/// What a GPU exporter's install did, as `up` and `host add` say it: no
/// line for a machine with no NVIDIA GPU (`none`).
pub(super) fn gpus_line(on: &str, outcome: &str) -> Option<String> {
    let said = match outcome.split_once(' ').map_or((outcome, ""), |(a, b)| (a, b)) {
        ("none", _) => return None,
        ("ours", _) => format!("nvidia_gpu_exporter {GPU_EXPORTER_VERSION} on :{GPU_EXPORTER_PORT}, a container of the habitat's ({GPU_EXPORTER_NAME})"),
        ("theirs", _) => format!("a GPU exporter already answers on :{GPU_EXPORTER_PORT}: scraped as it is"),
        ("skip", why) => format!("no GPU exporter: {why}"),
        _ => format!("no GPU exporter: the install said nothing it did ({outcome:?})"),
    };
    Some(format!("{:<8}{on}: {said}", "gpus"))
}

/// Whether this machine is Linux with an NVIDIA GPU (`nvidia-smi -L`
/// lists one), as an SSH host's probe asks it.
pub(super) fn nvidia_gpu_here() -> bool {
    cfg!(target_os = "linux") && Command::new("nvidia-smi").arg("-L").stdin(Stdio::null()).stderr(Stdio::null()).output().is_ok_and(|o| o.status.success() && lists_a_gpu(&String::from_utf8_lossy(&o.stdout)))
}

/// Whether `nvidia-smi -L` listed a GPU.
fn lists_a_gpu(listed: &str) -> bool {
    listed.lines().any(|l| l.starts_with("GPU "))
}

/// Whether an install's outcome leaves a GPU exporter answering there.
fn gpus_answer(outcome: &str) -> bool {
    matches!(outcome.split(' ').next(), Some("ours" | "theirs"))
}

/// What a container exporter's install did, as `up` and `host add` say it.
pub(super) fn containers_line(on: &str, outcome: &str) -> String {
    let said = match outcome.split_once(' ').map_or((outcome, ""), |(a, b)| (a, b)) {
        ("ours", _) => format!("cAdvisor {} on :{CADVISOR_PORT}, a container of the habitat's ({CADVISOR_NAME})", CADVISOR_IMAGE.rsplit(':').next().unwrap_or("")),
        ("theirs", _) => format!("a container exporter already answers on :{CADVISOR_PORT}: scraped as it is"),
        ("skip", why) => format!("no container exporter: {why}"),
        _ => format!("no container exporter: the install said nothing it did ({outcome:?})"),
    };
    format!("containers {on}: {said}")
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

/// This machine's exporters (its node's, its containers' and its GPUs'),
/// installed here unless the run says not to: what each did, and whether
/// a GPU exporter answers here now (`None` when none was installed: the
/// run said not to).
pub(super) fn install_here(label: &str) -> (Vec<String>, Option<bool>) {
    if node_exporter_off() {
        return (Vec::new(), None);
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    super::progress("installing the node, container and GPU exporters on this machine");
    let mut lines = vec![
        match run_here(&install_script(sudo_word(tty))) {
            Ok(o) => install_line(label, &o),
            Err(why) => format!("{:<8}{label}: no exporter: {why}", "node"),
        },
        match run_here(&containers_script(sudo_word(tty))) {
            Ok(o) => containers_line(label, &o),
            Err(why) => format!("containers {label}: no container exporter: {why}"),
        },
    ];
    let gpus = match run_here(&gpus_script(sudo_word(tty))) {
        Ok(o) => {
            lines.extend(gpus_line(label, &o));
            gpus_answer(&o)
        }
        Err(why) => {
            lines.push(format!("{:<8}{label}: no GPU exporter: {why}", "gpus"));
            false
        }
    };
    (lines, Some(gpus))
}

/// The exporters installed on `host` over the person's login, as `up
/// --host`, `host add` and `host probe` do it (its GPUs' only where it has
/// an NVIDIA GPU).
pub(super) fn install_over(host: &str, user: Option<&str>, port: Option<u16>) -> Vec<String> {
    let mut lines = vec![
        match super::remote::login_script(host, user, port, install_script) {
            Ok(o) => install_line(host, &o),
            Err(why) => format!("{:<8}{host}: no exporter: {why}", "node"),
        },
        match super::remote::login_script(host, user, port, containers_script) {
            Ok(o) => containers_line(host, &o),
            Err(why) => format!("containers {host}: no container exporter: {why}"),
        },
    ];
    match super::remote::login_script(host, user, port, gpus_script) {
        Ok(o) => lines.extend(gpus_line(host, &o)),
        Err(why) => lines.push(format!("{:<8}{host}: no GPU exporter: {why}", "gpus")),
    }
    lines
}

/// The habitat's exporters taken off `host` over the person's login: what
/// was removed, or why not.
pub(super) fn remove_over(host: &str, user: Option<&str>, port: Option<u16>) -> Vec<String> {
    let mut out = Vec::new();
    for (what, script) in [("node exporter", remove_script as fn(&str) -> String), ("container exporter", containers_remove_script), ("GPU exporter", gpus_remove_script)] {
        match super::remote::login_script(host, user, port, script) {
            Ok(o) if o == "removed" => out.push(format!("the habitat's {what} is removed from {host}")),
            Ok(_) => {}
            Err(why) => out.push(format!("the habitat's {what} is left on {host} ({why})")),
        }
    }
    out
}

/// The habitat's exporters taken off this machine: what was removed, or
/// why not.
pub(super) fn remove_here() -> Vec<String> {
    if node_exporter_off() {
        return Vec::new();
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut out = Vec::new();
    for (what, script) in [("node exporter", remove_script(sudo_word(tty))), ("container exporter", containers_remove_script(sudo_word(tty))), ("GPU exporter", gpus_remove_script(sudo_word(tty)))] {
        match run_here(&script) {
            Ok(o) if o == "removed" => out.push(format!("the habitat's {what} is removed from this machine")),
            Ok(_) => {}
            Err(why) => out.push(format!("the habitat's {what} is left on this machine ({why})")),
        }
    }
    out
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
    let containers = dir.join("containers");
    std::fs::create_dir_all(&containers).map_err(|e| format!("create {}: {e}", containers.display()))?;
    for (file, port) in [(dir.join("hosts.json"), NODE_EXPORTER_PORT), (containers.join("targets.json"), CADVISOR_PORT)] {
        if file.exists() {
            continue;
        }
        let text = serde_json::to_string_pretty(&serde_json::json!([
            { "targets": [format!("host.docker.internal:{port}")], "labels": { "host": label } }
        ]))
        .unwrap_or_default();
        std::fs::write(&file, format!("{text}\n")).map_err(|e| format!("write {}: {e}", file.display()))?;
    }
    Ok(())
}

/// The habitat's own machine among the GPU exporters the store scrapes:
/// `senses.d/gpus/local.json` (`host.docker.internal`, labelled `host`)
/// while one answers there, and no file while none does. The head writes
/// the hosts' beside it (`gpus/hosts.json`), from the hosts that offer an
/// NVIDIA GPU. `answers` is what this run's install said; a run that
/// installed nothing (the habitat's run inside its image on a machine
/// reached over SSH, whose exporters the person's login installs) asks
/// docker there instead (`gpu_exporter_expected_here`). Whether the file
/// changed.
pub(super) fn gpu_target_here(allocation: &Path, label: &str, answers: Option<bool>) -> Result<bool, String> {
    let dir = allocation.join("senses.d").join("gpus");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join("local.json");
    if !answers.unwrap_or_else(gpu_exporter_expected_here) {
        return Ok(std::fs::remove_file(&file).is_ok());
    }
    let text = serde_json::to_string_pretty(&serde_json::json!([
        { "targets": [format!("host.docker.internal:{GPU_EXPORTER_PORT}")], "labels": { "host": label } }
    ]))
    .unwrap_or_default();
    write_if_changed(&file, &format!("{text}\n"))
}

/// Whether docker here runs the habitat's GPU exporter, or could give one
/// the GPU (its NVIDIA runtime is registered: the exporter's install,
/// which the person's login runs after the habitat's `up` there, finds a
/// GPU to read).
fn gpu_exporter_expected_here() -> bool {
    let docker = |args: &[&str]| Command::new("docker").args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let name = format!("name=^{GPU_EXPORTER_NAME}$");
    let label = format!("label={GPU_EXPORTER_LABEL}");
    !docker(&["ps", "-aq", "--filter", &name, "--filter", &label]).trim().is_empty() || runtimes_offer_nvidia(&docker(&["info", "--format", "{{json .Runtimes}}"]))
}

/// Whether `docker info`'s runtimes (as JSON) name NVIDIA's.
fn runtimes_offer_nvidia(runtimes: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(runtimes.trim()).ok().and_then(|v| v.as_object().map(|m| m.contains_key("nvidia"))).unwrap_or(false)
}

/// The store's scrape job of the machines' GPUs, from `senses.d/gpus/`,
/// added to its configuration when it has none: whether it was.
pub(super) fn ensure_gpus_job(allocation: &Path) -> Result<bool, String> {
    let conf = allocation.join("senses.yml");
    let Ok(text) = std::fs::read_to_string(&conf) else { return Ok(false) };
    if text.contains("job_name: gpus") || !text.contains("scrape_configs:\n") {
        return Ok(false);
    }
    let job = "  # The machines' NVIDIA GPUs (nvidia_gpu_exporter), labelled `host`, from senses.d/gpus/.\n  - job_name: gpus\n    scrape_interval: 15s\n    file_sd_configs:\n      - files: [\"/etc/prometheus/senses.d/gpus/*.json\"]\n";
    let text = text.replacen("scrape_configs:\n", &format!("scrape_configs:\n{job}"), 1);
    std::fs::write(&conf, text).map_err(|e| format!("write {}: {e}", conf.display()))?;
    Ok(true)
}

/// The store's scrape job of the machines' containers, from
/// `senses.d/containers/`, added to its configuration when it has none:
/// whether it was.
pub(super) fn ensure_containers_job(allocation: &Path) -> Result<bool, String> {
    let conf = allocation.join("senses.yml");
    let Ok(text) = std::fs::read_to_string(&conf) else { return Ok(false) };
    // cAdvisor renders every container's series on each scrape, which on
    // a busy machine takes longer than the store's default 5 s: the job
    // waits up to 14 s (inside its 15 s interval), and a job an earlier
    // toolchain wrote is given that timeout too
    const EARLIER: &str = "  - job_name: containers\n    scrape_interval: 15s\n    file_sd_configs:\n";
    const NOW: &str = "  - job_name: containers\n    scrape_interval: 15s\n    scrape_timeout: 14s\n    file_sd_configs:\n";
    if text.contains(EARLIER) {
        std::fs::write(&conf, text.replacen(EARLIER, NOW, 1)).map_err(|e| format!("write {}: {e}", conf.display()))?;
        return Ok(true);
    }
    if text.contains("job_name: containers") || !text.contains("scrape_configs:\n") {
        return Ok(false);
    }
    const FILES: &str = "      - files: [\"/etc/prometheus/senses.d/containers/*.json\"]\n";
    let job = format!("  # The machines' containers (cAdvisor), labelled `host`, from senses.d/containers/.\n{NOW}{FILES}");
    let text = text.replacen("scrape_configs:\n", &format!("scrape_configs:\n{job}"), 1);
    std::fs::write(&conf, text).map_err(|e| format!("write {}: {e}", conf.display()))?;
    Ok(true)
}

/// The store's scrape job of the services the habitat placed on its hosts
/// (`hale habitat organism place`, a declaration's `readings`): their own
/// readings, from `senses.d/services/`, one file per placement the head
/// writes as placements are recorded (`dna/api/habitat_placed.hl`). Added
/// to the configuration when it has none, beside the containers' job:
/// whether it was.
pub(super) fn ensure_services_job(allocation: &Path) -> Result<bool, String> {
    let dir = allocation.join("senses.d").join("services");
    if allocation.join("senses.d").is_dir() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let conf = allocation.join("senses.yml");
    let Ok(text) = std::fs::read_to_string(&conf) else { return Ok(false) };
    if text.contains("job_name: services") || !text.contains("scrape_configs:\n") {
        return Ok(false);
    }
    let job = "  # The services the habitat placed on its hosts, each serving its own readings, labelled `host`, `organism`, `service` and `kind`, from senses.d/services/.\n  - job_name: services\n    scrape_interval: 15s\n    file_sd_configs:\n      - files: [\"/etc/prometheus/senses.d/services/*.json\"]\n";
    let text = text.replacen("scrape_configs:\n", &format!("scrape_configs:\n{job}"), 1);
    std::fs::write(&conf, text).map_err(|e| format!("write {}: {e}", conf.display()))?;
    Ok(true)
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
/// replaces the last; a file is written only when its content differs, and
/// the answer is whether any changed (or the old Containers dashboard
/// went), which `up` restarts a Grafana it leaves running for.
pub(super) fn write_provisioning(hab_dir: &Path, prometheus_url: &str) -> Result<bool, String> {
    let root = hab_dir.join("grafana").join("provisioning");
    let sources = root.join("datasources");
    let boards = root.join("dashboards");
    for d in [&sources, &boards, &hab_dir.join("grafana").join("data")] {
        std::fs::create_dir_all(d).map_err(|e| format!("create {}: {e}", d.display()))?;
    }
    let ds = format!(
        "apiVersion: 1\ndatasources:\n  - name: habitat senses\n    uid: habitat-senses\n    type: prometheus\n    access: proxy\n    url: {prometheus_url}\n    isDefault: true\n    editable: false\n    jsonData:\n      timeInterval: 15s\n"
    );
    let mut changed = write_if_changed(&sources.join("habitat.yaml"), &ds)?;
    changed |= write_if_changed(&boards.join("habitat.yaml"), DASHBOARD_PROVIDER)?;
    changed |= write_if_changed(&boards.join("hosts.json"), HOSTS_DASHBOARD)?;
    changed |= write_if_changed(&boards.join("services.json"), SERVICES_DASHBOARD)?;
    // the Containers dashboard it replaced, under the same uid: a second
    // file of that uid would be refused
    changed |= std::fs::remove_file(boards.join("containers.json")).is_ok();
    Ok(changed)
}

/// The habitat's dashboards' provider: the files beside it, read again
/// every ten seconds (Grafana's default, said rather than assumed), so a
/// running Grafana takes a changed dashboard on its own.
const DASHBOARD_PROVIDER: &str = "apiVersion: 1\nproviders:\n  - name: habitat\n    folder: habitat\n    type: file\n    disableDeletion: true\n    allowUiUpdates: false\n    updateIntervalSeconds: 10\n    options:\n      path: /etc/grafana/provisioning/dashboards\n";

/// `content` at `path` unless it is there already: whether it was written.
fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if std::fs::read(path).is_ok_and(|had| had == content.as_bytes()) {
        return Ok(false);
    }
    std::fs::write(path, content).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(true)
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
    s.push_str(&super::compose_labels("habitat", "grafana"));
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
        // its footprint: no plugin fetched at its start, none it never
        // uses loaded, no alerting of its own (the habitat's reflexes are
        // its senses' rules, read by the operator face), and the Go
        // runtime collecting under its soft limit
        ("GF_PLUGINS_PREINSTALL_DISABLED", "true".to_string()),
        ("GF_PLUGINS_DISABLE_PLUGINS", GRAFANA_UNUSED_PLUGINS.to_string()),
        ("GF_UNIFIED_ALERTING_ENABLED", "false".to_string()),
        ("GOMEMLIMIT", GRAFANA_GOMEMLIMIT.to_string()),
    ] {
        s.push_str(&format!("      {k}: \"{v}\"\n"));
    }
    s.push_str(&format!(
        "    volumes:\n      - \"{hab_dir}/grafana/provisioning:/etc/grafana/provisioning:ro\"\n      - \"{hab_dir}/grafana/data:/var/lib/grafana\"\n    mem_limit: {GRAFANA_MEM_LIMIT}\n    restart: on-failure\n"
    ));
    s
}

/// What the face is told of Grafana: its port and header, and, when it runs
/// on demand, the directory of the habitat's compose files (`dir`), with
/// which the face starts and stops it.
pub(super) fn grafana_face_env(port: u16, header: &str, on_demand: Option<&str>) -> String {
    let mut s = format!("      HALE_DNA_GRAFANA_PORT: \"{port}\"\n      HALE_DNA_GRAFANA_HEADER: \"{header}\"\n");
    if let Some(dir) = on_demand {
        s.push_str(&format!("      HALE_DNA_GRAFANA_ON_DEMAND: \"{dir}\"\n"));
    }
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
        assert!(s.contains("'{{.State.Running}} {{index .Config.Labels \"'$l'\"}}'"), "docker's template survives the format");
        // the container's version and the habitat whose it is are its label:
        // one made by another habitat, or another version, is replaced
        assert!(s.contains(&format!("want=\"{NODE_EXPORTER_VERSION} {}\"", super::super::habitat_label())), "the label is the version and the habitat while there are no flags");
        assert!(s.contains("--label 'hale.habitat=") && s.contains("--label hale.organism=habitat --label 'hale.role=exporter'"), "{s}");
    }

    #[test]
    fn both_scripts_are_shell() {
        for script in [install_script("sudo"), install_script("sudo -n"), remove_script("sudo -n"), gpus_script("sudo -n"), gpus_remove_script("sudo -n")] {
            let ok = std::process::Command::new("sh").args(["-n", "-c", &script]).status().expect("sh");
            assert!(ok.success(), "{script}");
        }
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
    fn provisioning_says_whether_a_dashboard_changed_and_grafana_polls_its_files() {
        let d = std::env::temp_dir().join(format!("hale-grafana-provisioning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let boards = d.join("grafana/provisioning/dashboards");
        assert!(write_provisioning(&d, "http://127.0.0.1:9090").unwrap(), "a first write changes everything");
        assert!(!write_provisioning(&d, "http://127.0.0.1:9090").unwrap(), "an `up` that changes nothing says so");
        let hosts = boards.join("hosts.json");
        let mtime = std::fs::metadata(&hosts).unwrap().modified().unwrap();
        assert!(!write_provisioning(&d, "http://127.0.0.1:9090").unwrap());
        assert_eq!(std::fs::metadata(&hosts).unwrap().modified().unwrap(), mtime, "an unchanged dashboard is not written again");
        // a toolchain's newer dashboard, or a moved data source: changed
        std::fs::write(&hosts, "{}").unwrap();
        assert!(write_provisioning(&d, "http://127.0.0.1:9090").unwrap());
        assert_eq!(std::fs::read_to_string(&hosts).unwrap(), HOSTS_DASHBOARD);
        assert!(write_provisioning(&d, "http://127.0.0.1:9091").unwrap());
        // the Containers dashboard Services replaced, left by an old toolchain
        std::fs::write(boards.join("containers.json"), "{}").unwrap();
        assert!(write_provisioning(&d, "http://127.0.0.1:9091").unwrap());
        assert!(!boards.join("containers.json").exists());
        assert!(!write_provisioning(&d, "http://127.0.0.1:9091").unwrap());
        // a running Grafana reads the dashboards' files again on its own
        let provider = std::fs::read_to_string(boards.join("habitat.yaml")).unwrap();
        assert!(provider.contains("\n    updateIntervalSeconds: 10\n"), "{provider}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn grafana_is_capped_and_trimmed() {
        let s = grafana_service("/h", 8805, "http://127.0.0.1:8800/", "X-Hale-Grafana-00", "");
        assert!(s.contains("      GOMEMLIMIT: \"200MiB\"\n"), "{s}");
        assert!(s.contains("    mem_limit: 384m\n"), "{s}");
        assert!(s.contains("GF_PLUGINS_PREINSTALL_DISABLED: \"true\""));
        assert!(s.contains("GF_UNIFIED_ALERTING_ENABLED: \"false\""));
        let disabled = s.lines().find_map(|l| l.trim().strip_prefix("GF_PLUGINS_DISABLE_PLUGINS: ")).expect("plugins disabled");
        assert!(!disabled.split(',').any(|p| p.trim_matches('"') == "prometheus"), "its data source stays: {disabled}");
    }

    #[test]
    fn grafana_runs_on_demand_unless_asked() {
        assert_eq!(grafana_runs_for(None), GrafanaRuns::OnDemand);
        assert_eq!(grafana_runs_for(Some("")), GrafanaRuns::OnDemand);
        assert_eq!(grafana_runs_for(Some("on")), GrafanaRuns::Always);
        assert_eq!(grafana_runs_for(Some(" off\n")), GrafanaRuns::Never);
        let on_demand = grafana_face_env(8805, "X-Hale-Grafana-00", Some("/h/allocations/abc"));
        assert!(on_demand.contains("HALE_DNA_GRAFANA_PORT: \"8805\"") && on_demand.contains("HALE_DNA_GRAFANA_HEADER: \"X-Hale-Grafana-00\""));
        assert!(on_demand.contains("      HALE_DNA_GRAFANA_ON_DEMAND: \"/h/allocations/abc\"\n"), "{on_demand}");
        let always = grafana_face_env(8805, "X-Hale-Grafana-00", None);
        assert!(!always.contains("ON_DEMAND"), "a Grafana that is always on is never stopped by the face: {always}");
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
    fn the_dashboards_are_json() {
        let v: serde_json::Value = serde_json::from_str(HOSTS_DASHBOARD).expect("hosts.json parses");
        assert_eq!(v["uid"], "hale-hosts");
        let v: serde_json::Value = serde_json::from_str(SERVICES_DASHBOARD).expect("services.json parses");
        assert_eq!((v["uid"].as_str(), v["title"].as_str()), (Some("hale-containers"), Some("Services")), "the Containers dashboard's uid, so its links still reach it");
        let hosts: serde_json::Value = serde_json::from_str(HOSTS_DASHBOARD).unwrap();
        assert!(hosts["links"].as_array().unwrap().iter().any(|l| l["title"] == "Services" && l["url"] == "/grafana/d/hale-containers"));
    }

    #[test]
    fn the_hosts_dashboard_opens_on_each_machines_use_now() {
        let v: serde_json::Value = serde_json::from_str(HOSTS_DASHBOARD).unwrap();
        let panels = v["panels"].as_array().unwrap();
        let top = &panels[0];
        assert_eq!((top["type"].as_str(), &top["gridPos"]["y"]), (Some("table"), &serde_json::json!(0)), "the table is first");
        let targets = top["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 14, "memory, disk: total, in use, free, used; CPU busy; GPU busy, memory, memory in use, temperature, power");
        for t in targets {
            let q = t["expr"].as_str().unwrap();
            assert!(t["instant"] == true && t["format"] == "table", "{q}");
            assert!(q.contains("by (host)") && q.contains("host=~\"$host\""), "one row a machine: {q}");
            assert_eq!(q.matches('(').count(), q.matches(')').count(), "{q}");
        }
        // both memory forms, as Memory used reads them, and the disk filter's file systems
        let exprs: Vec<&str> = targets.iter().map(|t| t["expr"].as_str().unwrap()).collect();
        for q in &exprs[..4] {
            assert!(q.contains(" or ") && (q.contains("node_memory_MemTotal_bytes") || q.contains("node_memory_MemAvailable_bytes")), "Linux: {q}");
            assert!(q.contains("node_memory_total_bytes") || q.contains("node_memory_active_bytes"), "macOS: {q}");
        }
        assert!(exprs[4..8].iter().all(|q| q.contains("mountpoint=~\"/|/System/Volumes/Data\"") && q.contains("fstype!~")));
        // the GPU columns, by nvidia_gpu_exporter's own names (its METRICS.md
        // and its recorded scrapes, a GB10's among them, which has no memory
        // series: the memory columns are blank there, never a made-up number)
        assert!(exprs[9].starts_with("100 * avg by (host) (nvidia_smi_utilization_gpu_ratio{"), "{}", exprs[9]);
        for (q, m) in [(exprs[10], "nvidia_smi_memory_total_bytes"), (exprs[11], "nvidia_smi_memory_used_bytes")] {
            // a host's memory only when every GPU of it reports its own
            assert!(q.starts_with(&format!("sum by (host) ({m}{{")) && q.contains(&format!("count by (host) ({m}{{")) && q.contains("== count by (host) (nvidia_smi_gpu_info{"), "{q}");
        }
        assert!(exprs[12].contains("max by (host) (nvidia_smi_temperature_gpu{") && exprs[13].contains("sum by (host) (nvidia_smi_power_draw_watts{"));
        let names = &top["transformations"][1]["options"]["renameByName"];
        let order = &top["transformations"][1]["options"]["indexByName"];
        for (i, (r, n)) in [("J", "GPU busy"), ("K", "GPU memory"), ("L", "GPU memory in use"), ("M", "GPU temperature"), ("N", "GPU power")].into_iter().enumerate() {
            assert_eq!((names[format!("Value #{r}")].as_str(), order[format!("Value #{r}")].as_u64()), (Some(n), Some(10 + i as u64)), "{r}");
        }
        let unit = |n: &str| top["fieldConfig"]["overrides"].as_array().unwrap().iter().find(|o| o["matcher"]["options"] == n).and_then(|o| o["properties"].as_array().unwrap().iter().find(|p| p["id"] == "unit")).map(|p| p["value"].clone());
        assert_eq!((unit("GPU busy"), unit("GPU temperature"), unit("GPU power")), (Some("percent".into()), Some("celsius".into()), Some("watt".into())));
        assert_eq!(top["transformations"][0]["id"], "merge");
        assert_eq!(top["transformations"][1]["options"]["excludeByName"]["Time"], true);
        // nothing overlaps: every panel's cells are its own
        let mut cells = std::collections::HashSet::new();
        for p in panels {
            let g = &p["gridPos"];
            let (x, y, w, h) = (g["x"].as_u64().unwrap(), g["y"].as_u64().unwrap(), g["w"].as_u64().unwrap(), g["h"].as_u64().unwrap());
            assert!(x + w <= 24, "{}", p["title"]);
            for cx in x..x + w {
                for cy in y..y + h {
                    assert!(cells.insert((cx, cy)), "{} overlaps another panel", p["title"]);
                }
            }
        }
        let ids: std::collections::HashSet<_> = panels.iter().map(|p| p["id"].as_u64().unwrap()).collect();
        assert_eq!(ids.len(), panels.len(), "panel ids are unique");
    }

    #[test]
    fn the_container_exporter_keeps_only_compose_labels_and_is_shell() {
        let s = containers_script("sudo -n");
        assert!(s.contains("-whitelisted_container_labels=com.docker.compose.project,com.docker.compose.service,hale.habitat,hale.organism,hale.role") && s.contains("-store_container_labels=false"), "{s}");
        assert!(s.contains(&format!("want=\"{CADVISOR_IMAGE} {CONTAINER_LABELS} ")), "a cAdvisor carrying other labels is replaced: {s}");
        assert!(s.contains("'{{.State.Running}} {{index .Config.Labels \"'$l'\"}}'"), "{s}");
        for script in [s, containers_remove_script("sudo")] {
            assert!(std::process::Command::new("sh").args(["-n", "-c", &script]).status().unwrap().success(), "{script}");
        }
        assert_eq!(outcome_of("x\nhale-containers: ours\n"), "ours");
    }

    #[test]
    fn the_gpu_exporter_pins_its_image_and_is_shell() {
        let s = gpus_script("sudo -n");
        assert!(GPU_EXPORTER_IMAGE.starts_with(&format!("utkuozdemir/nvidia_gpu_exporter:{GPU_EXPORTER_VERSION}@sha256:")) && GPU_EXPORTER_IMAGE.len() == "utkuozdemir/nvidia_gpu_exporter:@sha256:".len() + GPU_EXPORTER_VERSION.len() + 64);
        assert!(s.contains(&format!("img={GPU_EXPORTER_IMAGE};")) && s.contains(&format!("p={GPU_EXPORTER_PORT};")), "{s}");
        assert!(s.contains("--gpus all -e NVIDIA_DRIVER_CAPABILITIES=utility \"$img\" --web.listen-address=\":$p\""), "{s}");
        assert!(s.contains(&format!("want=\"{GPU_EXPORTER_IMAGE} {}\"", super::super::habitat_label())), "one made by another habitat, or from another image, is replaced: {s}");
        assert!(s.contains("--label hale.organism=habitat --label 'hale.role=exporter'"), "{s}");
        assert!(s.contains("'{{.State.Running}} {{index .Config.Labels \"'$l'\"}}'"), "{s}");
        let r = gpus_remove_script("sudo");
        assert!(r.contains("--filter \"label=$l\""), "only the habitat's own is removed: {r}");
        for script in [s, gpus_script("sudo"), r] {
            assert!(std::process::Command::new("sh").args(["-n", "-c", &script]).status().unwrap().success(), "{script}");
        }
        assert_eq!(outcome_of("x\nhale-gpus: skip docker gives no container the GPU\n"), "skip docker gives no container the GPU");
        assert_eq!(gpus_line("box", "none no NVIDIA GPU"), None, "a machine with no GPU says nothing of one");
        assert!(gpus_line("box", "ours").unwrap().starts_with(&format!("gpus    box: nvidia_gpu_exporter {GPU_EXPORTER_VERSION} on :9835")));
        assert!(gpus_line("box", "theirs").unwrap().ends_with("scraped as it is"));
        assert!(gpus_answer("ours") && gpus_answer("theirs") && !gpus_answer("skip x") && !gpus_answer("none Darwin") && !gpus_answer(""));
        assert!(lists_a_gpu("GPU 0: NVIDIA GB10 (UUID: GPU-0)\n") && !lists_a_gpu("No devices were found\n") && !lists_a_gpu(""));
        assert!(runtimes_offer_nvidia("{\"io.containerd.runc.v2\":{\"path\":\"runc\"},\"nvidia\":{\"path\":\"nvidia-container-runtime\"}}\n"));
        assert!(!runtimes_offer_nvidia("{\"runc\":{\"path\":\"runc\"}}") && !runtimes_offer_nvidia(""));
    }

    /// The GPU exporter's install run against stand-ins for `uname`,
    /// `nvidia-smi`, `curl` and `docker` (the docker one records what it
    /// was asked): its outcome, and docker's arguments.
    fn gpus_run(case: &str, os: &str, gpu: bool, run_ok: bool) -> (String, String) {
        let d = std::env::temp_dir().join(format!("hale-gpus-{}-{case}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let log = d.join("docker.log");
        let stub = |name: &str, body: &str| {
            use std::os::unix::fs::PermissionsExt;
            let f = d.join(name);
            std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        stub("uname", &format!("echo {os}"));
        stub("nvidia-smi", if gpu { "echo 'GPU 0: NVIDIA GB10 (UUID: GPU-0)'" } else { "echo 'No devices were found'; exit 6" });
        stub("curl", "exit 7");
        let run = if run_ok { "echo 0123abc" } else { "echo 'docker: Error response from daemon: could not select device driver \"\" with capabilities: [[gpu]]' >&2; exit 125" };
        stub("docker", &format!("echo \"$*\" >> '{}'\ncase \"$1\" in info|rm) exit 0 ;; inspect) exit 1 ;; run) {run} ;; esac", log.display()));
        let out = std::process::Command::new("sh").args(["-c", &gpus_script("sudo -n")]).env("PATH", format!("{}:/usr/bin:/bin", d.display())).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let said = (outcome_of(&String::from_utf8_lossy(&out.stdout)), std::fs::read_to_string(&log).unwrap_or_default());
        std::fs::remove_dir_all(&d).ok();
        said
    }

    #[test]
    fn the_gpu_exporter_runs_only_where_an_nvidia_gpu_is_and_docker_can_give_it() {
        assert_eq!(gpus_run("mac", "Darwin", false, true), ("none Darwin".to_string(), String::new()), "a Mac runs none, and docker is never asked");
        assert_eq!(gpus_run("nogpu", "Linux", false, true), ("none no NVIDIA GPU".to_string(), String::new()));
        let (said, asked) = gpus_run("gpu", "Linux", true, true);
        assert_eq!(said, "ours");
        let run = asked.lines().find(|l| l.starts_with("run ")).expect("docker run");
        assert!(run.contains(&format!("--name hale-gpu-exporter --label lang.hale.gpu-exporter={GPU_EXPORTER_IMAGE} ")) && run.contains(&format!("--net host --gpus all -e NVIDIA_DRIVER_CAPABILITIES=utility {GPU_EXPORTER_IMAGE} --web.listen-address=:9835")), "{run}");
        // no NVIDIA Container Toolkit: said, and nothing is left behind
        let (said, asked) = gpus_run("notoolkit", "Linux", true, false);
        assert!(said.starts_with("skip docker gives no container the GPU (the NVIDIA Container Toolkit): docker: Error response from daemon: could not select device driver"), "{said}");
        assert_eq!(asked.lines().last(), Some("rm -f hale-gpu-exporter"), "{asked}");
    }

    #[test]
    fn the_habitats_own_machine_is_among_the_gpus_while_one_answers_there() {
        let d = std::env::temp_dir().join(format!("hale-gpus-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let local = d.join("senses.d/gpus/local.json");
        assert!(!gpu_target_here(&d, "box", Some(false)).unwrap() && !local.exists(), "none: no file");
        assert!(gpu_target_here(&d, "box", Some(true)).unwrap());
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&local).unwrap()).unwrap();
        assert_eq!(v, serde_json::json!([{ "targets": ["host.docker.internal:9835"], "labels": { "host": "box" } }]));
        assert!(!gpu_target_here(&d, "box", Some(true)).unwrap(), "unchanged: not written again");
        assert!(gpu_target_here(&d, "box", Some(false)).unwrap() && !local.exists(), "gone with the exporter");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_gpus_job_is_added_once() {
        let d = std::env::temp_dir().join(format!("hale-gpus-job-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("senses.yml"), "global:\n  scrape_interval: 5s\nscrape_configs:\n  - job_name: spine\n").unwrap();
        assert!(ensure_gpus_job(&d).unwrap());
        assert!(!ensure_gpus_job(&d).unwrap(), "once");
        let t = std::fs::read_to_string(d.join("senses.yml")).unwrap();
        assert!(t.contains("  - job_name: gpus\n    scrape_interval: 15s\n    file_sd_configs:\n      - files: [\"/etc/prometheus/senses.d/gpus/*.json\"]\n") && t.contains("  - job_name: spine"), "{t}");
        let e = d.join("none");
        std::fs::create_dir_all(&e).unwrap();
        assert!(!ensure_gpus_job(&e).unwrap(), "no configuration yet: nothing");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_services_dashboard_reads_both_kinds_in_one_table() {
        let v: serde_json::Value = serde_json::from_str(SERVICES_DASHBOARD).unwrap();
        let panels = v["panels"].as_array().unwrap();
        let exprs = |p: &serde_json::Value| p["targets"].as_array().unwrap().iter().map(|t| t["expr"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let table = panels.iter().find(|p| p["title"] == "Running now").expect("the table");
        let queries = exprs(table);
        assert_eq!(queries.len(), 4, "state, up for, memory and CPU");
        for q in &queries {
            // both kinds, under the same labels, joined by `or`
            assert!(q.contains("job=\"containers\"") && q.contains("job=\"services\"") && q.contains(" or "), "{q}");
            assert!(q.contains("\"kind\", \"container\"") && q.contains("\"name\", \"$1\", \"service\""), "{q}");
            assert!(q.contains("by (kind, host, organism, part, name, image)"), "{q}");
            assert_eq!(q.matches('(').count(), q.matches(')').count(), "{q}");
        }
        // a placed service that does not answer is `up` 0: down, in the table
        assert!(queries[0].contains("up{job=\"services\""), "{}", queries[0]);
        assert!(queries[1].contains("time() - hale_process_start_time_seconds{job=\"services\""));
        assert!(queries[2].contains("hale_process_resident_memory_bytes{job=\"services\""));
        assert!(queries[3].contains("rate(hale_process_cpu_seconds_total{job=\"services\""));
        assert!(table["transformations"][0]["id"] == "merge");
        for title in ["CPU", "Memory"] {
            let p = panels.iter().find(|p| p["title"] == title).unwrap();
            let q = &exprs(p)[0];
            assert!(q.contains("job=\"containers\"") && q.contains("job=\"services\""), "{title}: {q}");
        }
        // the variables: hosts and organisms of both kinds; the scope filters containers alone
        let vars = v["templating"]["list"].as_array().unwrap();
        let def = |n: &str| vars.iter().find(|x| x["name"] == n).unwrap()["definition"].as_str().unwrap().to_string();
        assert!(def("host").contains("containers|services"));
        assert!(def("organism").contains("job=\"services\"") && def("organism").contains("job=\"containers\""));
        for p in panels {
            for q in exprs(p) {
                assert!(!q.contains("job=\"services\"") || !q.split("job=\"services\"").nth(1).unwrap_or("").split('}').next().unwrap_or("").contains("$scope"), "a placed service is always the habitat's: {q}");
            }
        }
    }

    #[test]
    fn the_services_job_is_added_once() {
        let d = std::env::temp_dir().join(format!("hale-services-job-{}", std::process::id()));
        std::fs::create_dir_all(d.join("senses.d")).unwrap();
        std::fs::write(d.join("senses.yml"), "global:\n  scrape_interval: 5s\nscrape_configs:\n  - job_name: spine\n").unwrap();
        assert!(ensure_containers_job(&d).unwrap());
        assert!(ensure_services_job(&d).unwrap());
        assert!(!ensure_services_job(&d).unwrap(), "once");
        let t = std::fs::read_to_string(d.join("senses.yml")).unwrap();
        assert_eq!(t.matches("job_name: services").count(), 1);
        assert!(t.contains("  - job_name: services\n    scrape_interval: 15s\n    file_sd_configs:\n      - files: [\"/etc/prometheus/senses.d/services/*.json\"]\n"), "{t}");
        assert!(t.contains("job_name: containers") && t.contains("  - job_name: spine"), "{t}");
        assert!(d.join("senses.d/services").is_dir(), "the directory the head writes into");
        // no configuration yet (the store is not made): nothing
        let e = d.join("none");
        std::fs::create_dir_all(&e).unwrap();
        assert!(!ensure_services_job(&e).unwrap());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_containers_job_is_added_once() {
        let d = std::env::temp_dir().join(format!("hale-containers-job-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("senses.yml"), "global:\n  scrape_interval: 5s\nscrape_configs:\n  - job_name: spine\n").unwrap();
        assert!(ensure_containers_job(&d).unwrap());
        assert!(!ensure_containers_job(&d).unwrap());
        let t = std::fs::read_to_string(d.join("senses.yml")).unwrap();
        assert!(t.contains("scrape_configs:\n  # The machines' containers") && t.contains("  - job_name: spine"), "{t}");
        assert!(t.contains("    scrape_timeout: 14s\n"), "cAdvisor's scrape waits past the store's default: {t}");
        // a job an earlier toolchain wrote is given the timeout, once
        std::fs::write(d.join("senses.yml"), "scrape_configs:\n  - job_name: containers\n    scrape_interval: 15s\n    file_sd_configs:\n      - files: [\"x\"]\n").unwrap();
        assert!(ensure_containers_job(&d).unwrap());
        assert!(!ensure_containers_job(&d).unwrap());
        assert!(std::fs::read_to_string(d.join("senses.yml")).unwrap().contains("scrape_interval: 15s\n    scrape_timeout: 14s\n"));
        std::fs::remove_dir_all(&d).ok();
    }
}
