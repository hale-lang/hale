//! `hale habitat up --host <name>`: the habitat on another machine.
//!
//! The habitat runs where its host is: its genome, record, memory, vault
//! and compose project live on that machine, brought up there by the same
//! `hale habitat up` that brings a local habitat up, run in the habitat's
//! toolchain image over the person's own SSH login. The machine needs
//! docker and nothing else. The admin machine keeps a pointer in the
//! local habitat's directory: the habitat's name, the host, the head's URL
//! (`http://<host>:<port>`, also `head.url`), the head's launch token
//! (`face-state/head.token`) and the record's identity. Every verb on the
//! admin machine reaches the head through that pointer; the verbs that
//! act on the habitat's machine itself (`up`, `down`) run there over SSH.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use super::{default_habitat_image_tag, face_state_dir, head_url_file, http_surface_call, manifest_pin_in_text, usage_text, HABITAT_PIN, HALE_GIT};
use crate::dna::local_habitat_dir;

/// The pointer's file, in the local habitat's directory.
const POINTER: &str = "remote.json";

/// The variables a container on the habitat's machine is told its address
/// by: the address its person-facing listeners bind (`LISTEN`) and the
/// name the habitat is reached by (`ADDRESS`).
pub(super) const LISTEN_ENV: &str = "HALE_DNA_HABITAT_LISTEN";
pub(super) const ADDRESS_ENV: &str = "HALE_DNA_HABITAT_ADDRESS";
/// The record a run of `hale habitat` on the machine must find there
/// before it stops or removes anything (`down`): the pointer's identity.
pub(super) const EXPECT_ENV: &str = "HALE_DNA_HABITAT_EXPECT";

/// What the person's environment carries to the containers on the
/// habitat's machine, as a local `up` hands it to its body and face.
const FORWARDED: &[&str] = &[
    "HALE_DNA_DISCOVER",
    "HALE_DNA_GRAFANA",
    "HALE_DNA_TRUSTED_LOCAL",
    "HALE_DNA_WAIT_SCALE",
    "HALE_DNA_FORGE",
    "HALE_DNA_COMPOSE",
    "HALE_DNA_SUITE_TAG",
    "LOTUS_DRAIN_GRACE_MS",
    "HALE_DNA_MEMORY_DSN_OWNER",
    "HALE_DNA_NATS_URL_OWNER",
];

fn pointer_path() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join(POINTER)
}

/// The pointer to a habitat on another machine, when this machine holds one.
pub(super) fn pointer() -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(pointer_path()).ok()?;
    serde_json::from_str(&text).ok().filter(|v: &serde_json::Value| v["host"].as_str().is_some_and(|h| !h.is_empty()))
}

/// The refusal of a verb that acts on the habitat's genome on this machine
/// while the habitat runs on another.
pub(super) fn refuse_local(verb: &str) -> Option<ExitCode> {
    let p = pointer()?;
    eprintln!(
        "hale habitat {verb}: the habitat `{}` runs on {} (`hale habitat up --host {}`), and `{verb}` acts on a habitat's genome on this machine; it has no route through the head",
        p["name"].as_str().unwrap_or(""),
        p["host"].as_str().unwrap_or(""),
        p["host"].as_str().unwrap_or("")
    );
    Some(ExitCode::from(2))
}

/// How the person reaches the machine over SSH: their own login.
#[derive(Clone, Default)]
struct Target {
    host: String,
    user: Option<String>,
    port: Option<u16>,
}

impl Target {
    fn of(p: &serde_json::Value) -> Target {
        Target {
            host: p["host"].as_str().unwrap_or("").to_string(),
            user: p["user"].as_str().filter(|u| !u.is_empty()).map(str::to_string),
            port: p["ssh_port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0),
        }
    }
    fn ssh(&self, tty: bool) -> Command {
        let mut c = Command::new("ssh");
        if let Some(cfg) = std::env::var("HALE_HABITAT_SSH_CONFIG").ok().filter(|v| !v.is_empty()) {
            c.args(["-F", &cfg]);
        }
        c.args(["-o", "BatchMode=yes"]);
        if tty {
            c.arg("-tt");
        }
        if let Some(p) = self.port {
            c.args(["-p", &p.to_string()]);
        }
        if let Some(u) = &self.user {
            c.args(["-l", u]);
        }
        c.arg(&self.host);
        c
    }
    /// `script` run by the login shell there: its stdout, or why not.
    fn run(&self, script: &str) -> Result<String, String> {
        let out = self.ssh(false).arg(script).stdin(Stdio::null()).output().map_err(|e| format!("ssh: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!("ssh {}: {}", self.host, String::from_utf8_lossy(&out.stderr).trim()))
        }
    }
}

/// A script of the habitat's (`readings`) run on `host` over the person's
/// own login, made with the `sudo` that suits the run: at a terminal the
/// session has one (sudo asks there), its output shown as it comes and its
/// outcome line kept back; elsewhere `sudo -n` and the output captured.
/// The outcome, or why it failed.
pub(super) fn login_script(host: &str, user: Option<&str>, port: Option<u16>, script: impl Fn(&str) -> String) -> Result<String, String> {
    use std::io::{Read, Write};
    let t = Target { host: host.to_string(), user: user.map(str::to_string), port };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut child = t
        .ssh(tty)
        .arg(script(super::readings::sudo_word(tty)))
        .stdin(if tty { Stdio::inherit() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(if tty { Stdio::inherit() } else { Stdio::piped() })
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;
    let mut all = String::new();
    if let Some(mut out) = child.stdout.take() {
        let mut pending = String::new();
        let mut buf = [0u8; 4096];
        let mut shown = std::io::stdout();
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
            all.push_str(&chunk);
            if !tty {
                continue;
            }
            // a terminal's: every line but the outcome, and a prompt (a
            // line not ended yet that cannot be the outcome) at once
            pending.push_str(&chunk);
            while let Some(i) = pending.find('\n') {
                let line: String = pending.drain(..=i).collect();
                let bare = line.trim_start_matches('\r');
                if !bare.starts_with("hale-node:") && !bare.starts_with("hale-containers:") {
                    let _ = shown.write_all(line.as_bytes());
                }
            }
            let p = pending.trim_start_matches('\r');
            let held = ["hale-node:", "hale-containers:"].iter().any(|m| m.starts_with(p) || p.starts_with(m));
            if !pending.is_empty() && !held {
                let _ = shown.write_all(pending.as_bytes());
                pending.clear();
            }
            let _ = shown.flush();
        }
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| format!("ssh: {e}"))?;
    if status.success() {
        Ok(super::readings::outcome_of(&all))
    } else if status.code() == Some(255) {
        Err(format!("ssh {host} did not connect{}", if err.trim().is_empty() { String::new() } else { format!(": {}", err.trim()) }))
    } else {
        Err(super::readings::last_words(&err, &all))
    }
}

/// The habitat's key on a host it places on, put there or taken off over
/// the person's own SSH login, as `up --host` reaches a machine: the line
/// `add_host` answers, appended to `user`'s `~/.ssh/authorized_keys` once,
/// or every line carrying the habitat's comment (`habitat-<key>`) removed.
/// The habitat's own key never authorizes itself.
pub(super) fn authorize_key(host: &str, user: &str, port: Option<u16>, line: &str) -> Result<(), String> {
    let t = Target { host: host.to_string(), user: Some(user.to_string()).filter(|u| !u.is_empty()), port };
    t.run(&authorize_script(line)).map(|_| ())
}

fn authorize_script(line: &str) -> String {
    format!(
        "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && {{ grep -qxF {l} ~/.ssh/authorized_keys || printf '%s\\n' {l} >> ~/.ssh/authorized_keys; }}",
        l = sq(line)
    )
}

pub(super) fn unauthorize_key(host: &str, user: &str, port: Option<u16>, comment: &str) -> Result<(), String> {
    let t = Target { host: host.to_string(), user: Some(user.to_string()).filter(|u| !u.is_empty()), port };
    t.run(&unauthorize_script(comment)).map(|_| ())
}

fn unauthorize_script(comment: &str) -> String {
    format!(
        "f=~/.ssh/authorized_keys; [ -f \"$f\" ] || exit 0; awk -v c={c} '$NF != c' \"$f\" > \"$f.hale.$$\" && cat \"$f.hale.$$\" > \"$f\"; rm -f \"$f.hale.$$\"",
        c = sq(comment)
    )
}

/// The account a habitat logs into a host as, when the person names none:
/// one of its own (`hale-<allocation key>`), never the person's, made over
/// their login (`provision_user`) and removed with the host
/// (`remove_user`). One per habitat, so two habitats on one host never
/// share a login, and one's teardown never removes the other's.
pub(super) fn habitat_user(identity: &str) -> String {
    format!("hale-{}", &identity[..12.min(identity.len())])
}

/// Whether `user` is an account the habitat made (`habitat_user`).
pub(super) fn is_habitat_user(user: &str) -> bool {
    user.strip_prefix("hale-").is_some_and(|k| k.len() == 12 && k.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The habitat's own account on a host, made over the person's login with
/// their sudo (asked at their terminal), with no sudo of its own and the
/// habitat's key its only `authorized_keys` line. On Linux a system user
/// with its own home, in the `docker` group (which is root-equivalent: this
/// keeps the habitat out of the person's account, not out of the host). On
/// macOS a hidden user (`dscl`, a uid below 500, no password, so nothing
/// but the key logs it in) admitted to Remote Login when Remote Login is
/// limited to a group; docker there is the person's own and stays theirs,
/// so a Mac host offers the habitat no compose. Any other system is
/// refused by name.
pub(super) fn provision_user(host: &str, port: Option<u16>, user: &str, line: &str) -> Result<(), String> {
    let t = Target { host: host.to_string(), user: None, port };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let ran = t
        .ssh(tty)
        .arg(provision_script(user, line, tty))
        .stdin(if tty { Stdio::inherit() } else { Stdio::null() })
        .status()
        .map_err(|e| format!("ssh: {e}"))?;
    if ran.success() {
        Ok(())
    } else {
        Err(format!("making the account {user} there failed (above){}", if tty { "" } else { "; run it at a terminal, where sudo can ask" }))
    }
}

fn provision_script(user: &str, line: &str, tty: bool) -> String {
    let sudo = if tty { "sudo" } else { "sudo -n" };
    format!(
        r#"set -e; u={u}; l={l}; S="{s}"
case "$(uname -s)" in
Linux)
  command -v useradd >/dev/null || {{ echo 'this host has no useradd: --user names an account you made' >&2; exit 1; }}
  id "$u" >/dev/null 2>&1 || $S useradd --system --create-home --shell /bin/sh "$u"
  if getent group docker >/dev/null; then $S usermod -aG docker "$u"; fi
  h=$(getent passwd "$u" | cut -d: -f6); g=$(id -gn "$u") ;;
Darwin)
  if ! id "$u" >/dev/null 2>&1; then
    taken=$(dscl . -list /Users UniqueID | awk '{{print $2}}')
    uid=401; while [ $uid -lt 500 ] && printf '%s\n' "$taken" | grep -qx "$uid"; do uid=$((uid + 1)); done
    [ $uid -lt 500 ] || {{ echo 'no free uid below 500 for a hidden account' >&2; exit 1; }}
    $S dscl . -create "/Users/$u"
    $S dscl . -create "/Users/$u" UniqueID "$uid"
    $S dscl . -create "/Users/$u" PrimaryGroupID 20
    $S dscl . -create "/Users/$u" UserShell /bin/sh
    $S dscl . -create "/Users/$u" RealName "the hale habitat's account"
    $S dscl . -create "/Users/$u" NFSHomeDirectory "/Users/$u"
    $S dscl . -create "/Users/$u" Password '*'
    $S dscl . -create "/Users/$u" IsHidden 1
    $S install -d -m 755 -o "$u" -g staff "/Users/$u"
  fi
  if dscl . -read /Groups/com.apple.access_ssh >/dev/null 2>&1; then $S dseditgroup -o edit -a "$u" -t user com.apple.access_ssh; fi
  h=$(dscl . -read "/Users/$u" NFSHomeDirectory | awk '{{print $2}}'); g=staff ;;
*) echo "$(uname -s) is neither Linux nor macOS: --user names an account you made" >&2; exit 1 ;;
esac
$S install -d -m 700 -o "$u" -g "$g" "$h/.ssh"
$S sh -c 'f="$1/.ssh/authorized_keys"; touch "$f"; grep -qxF "$2" "$f" || printf "%s\n" "$2" >> "$f"; chown "$3" "$f"; chmod 600 "$f"' sh "$h" "$l" "$u""#,
        u = sq(user),
        l = sq(line),
        s = sudo
    )
}

/// The habitat's own account taken off a host, its home with it, over the
/// person's login with their sudo; nothing when it is not there.
pub(super) fn remove_user(host: &str, port: Option<u16>, user: &str) -> Result<(), String> {
    if !is_habitat_user(user) {
        return Err(format!("{user} is not an account the habitat made"));
    }
    let t = Target { host: host.to_string(), user: None, port };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let sudo = if tty { "sudo" } else { "sudo -n" };
    let ran = t
        .ssh(tty)
        .arg(remove_user_script(user, sudo))
        .stdin(if tty { Stdio::inherit() } else { Stdio::null() })
        .status()
        .map_err(|e| format!("ssh: {e}"))?;
    if ran.success() {
        Ok(())
    } else {
        Err(format!("removing the account {user} there failed{}", if tty { "" } else { " (sudo needs a terminal)" }))
    }
}

/// The account removed, its sessions ended first: the habitat's own SSH
/// session there, and on Linux the user manager logind started for it, are
/// still exiting just after the habitat's last act, and `userdel` refuses a
/// user with a process. Retried a few seconds; its home goes with it. On
/// macOS the record is deleted with `dscl` and its home under /Users with
/// it, and it leaves the Remote Login group.
fn remove_user_script(user: &str, sudo: &str) -> String {
    format!(
        r#"u={u}; S="{s}"; id "$u" >/dev/null 2>&1 || exit 0
case "$(uname -s)" in
Darwin)
  $S pkill -KILL -u "$u" 2>/dev/null
  h=$(dscl . -read "/Users/$u" NFSHomeDirectory 2>/dev/null | awk '{{print $2}}')
  if dscl . -read /Groups/com.apple.access_ssh >/dev/null 2>&1; then $S dseditgroup -o edit -d "$u" -t user com.apple.access_ssh 2>/dev/null; fi
  $S dscl . -delete "/Users/$u" || exit 1
  case "$h" in "/Users/$u") $S rm -rf "$h" ;; esac ;;
*)
  $S loginctl terminate-user "$u" 2>/dev/null; $S pkill -KILL -u "$u" 2>/dev/null
  for _ in 1 2 3 4 5 6 7 8 9 10; do $S userdel -r "$u" 2>/dev/null && exit 0; sleep 1; done; $S userdel -r "$u" ;;
esac"#,
        u = sq(user),
        s = sudo
    )
}

/// The comment the habitat's public key carries (`habitat-<allocation
/// key>`, the record identity's first 12 hex digits).
pub(super) fn key_comment(identity: &str) -> String {
    format!("habitat-{}", &identity[..12.min(identity.len())])
}

/// A word for the remote shell, single-quoted.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// What the machine says of itself: its user's home, uid, gid and name.
/// The docker socket's group is not among them: it is the daemon's, read
/// where the run there mounts it (`in_image`), since the login's shell may
/// see another socket or none (a VM's docker, as Colima's on macOS).
struct Machine {
    home: String,
    uid: String,
    gid: String,
    user: String,
    /// `uname -s` of the login's shell: `Darwin` when the machine's docker
    /// runs in a VM (Colima, Docker Desktop), whose containers never own
    /// the machine's own addresses.
    os: String,
}

fn probe(t: &Target) -> Result<Machine, String> {
    let said = t.run("printf '%s\\n' \"$HOME\"; id -u; id -g; id -un; uname -s; docker version --format '{{.Server.Version}}' >/dev/null 2>&1 && echo docker")?;
    let lines: Vec<&str> = said.lines().map(str::trim).collect();
    if lines.len() < 6 || lines[5] != "docker" {
        return Err(format!("{} answers over SSH, but its docker does not (the habitat's machine needs docker and the user's access to its socket)", t.host));
    }
    if lines[0].is_empty() || !lines[0].starts_with('/') {
        return Err(format!("{} names no home directory for its user", t.host));
    }
    Ok(Machine { home: lines[0].to_string(), uid: lines[1].to_string(), gid: lines[2].to_string(), user: lines[3].to_string(), os: lines[4].to_string() })
}

/// The habitat's directory on its machine and what holds it (the user's
/// data directory, mounted into every container run there).
fn data_dir(m: &Machine) -> String {
    format!("{}/.local/share/hale", m.home)
}

fn habitat_dir(m: &Machine) -> String {
    format!("{}/habitat/local", data_dir(m))
}

/// The address the habitat's person-facing listeners bind on the machine:
/// the address its name resolves to here, which a Linux docker's host
/// network owns; on a machine whose docker runs in a VM (macOS), every
/// interface of the VM, since the VM owns none of the machine's addresses
/// and its port forwarding publishes what listens there on the machine.
fn listen_on(os: &str, resolved: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    if os == "Darwin" {
        return Ok("0.0.0.0".to_string());
    }
    resolved()
}

/// The address the machine is reached at from here, as this machine
/// resolves its name.
fn listen_address(host: &str) -> Result<String, String> {
    super::ipv4_or_first(host, &super::resolved(host)?)
}

/// How the toolchain image reaches the machine, from the platforms
/// (`<os>/<arch>`) of the machine's docker, of the tag there ("" when it
/// has none) and of the tag here ("" when this docker has none), and
/// whether this toolchain is a release's build: there already; sent from
/// this machine's docker (`docker save | docker load`); for a release's
/// build, the published image pulled for the machine's platform; else built
/// there from this toolchain's commit. A tag of another platform is never
/// used or sent, since the machine could not run it without emulation.
#[derive(Debug, PartialEq)]
enum ImageRoute {
    There,
    Send,
    Pull { mismatch: Option<String> },
    Build { mismatch: Option<String> },
}

fn image_route(machine: &str, there: &str, here: &str, release: bool) -> ImageRoute {
    if !there.is_empty() && there == machine {
        return ImageRoute::There;
    }
    if !here.is_empty() && here == machine {
        return ImageRoute::Send;
    }
    let mismatch = [there, here].iter().find(|p| !p.is_empty()).map(|p| p.to_string());
    if release {
        ImageRoute::Pull { mismatch }
    } else {
        ImageRoute::Build { mismatch }
    }
}

/// The commit a tag names, from `git ls-remote <repo> refs/tags/<tag>
/// refs/tags/<tag>^{}`: an annotated tag's peeled line, else the tag's own.
fn tag_commit(ls_remote: &str, tag: &str) -> Option<String> {
    let own = format!("refs/tags/{tag}");
    let peeled = format!("refs/tags/{tag}^{{}}");
    let mut found = None;
    for (sha, name) in ls_remote.lines().filter_map(|l| l.split_once('\t')) {
        if name.trim() == peeled {
            return Some(sha.trim().to_string());
        }
        if name.trim() == own {
            found = Some(sha.trim().to_string());
        }
    }
    found
}

/// Whether this toolchain is the build of its version's release: the
/// release tag `v<version>` on the toolchain's repository names this
/// build's commit. A build from any other commit shares the version string
/// and is never one; so is any build when the tag cannot be read.
fn is_release_build() -> bool {
    let pin = HABITAT_PIN.trim();
    if pin.is_empty() {
        return false;
    }
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let out = Command::new("git")
        .args(["ls-remote", HALE_GIT, &format!("refs/tags/{tag}"), &format!("refs/tags/{tag}^{{}}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => tag_commit(&String::from_utf8_lossy(&o.stdout), &tag).is_some_and(|c| c == pin),
        _ => false,
    }
}

/// `<os>/<arch>` of a docker answer, `amd64` and `x86_64` alike.
fn platform(os: &str, arch: &str) -> String {
    let arch = match arch.trim() {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    };
    format!("{}/{arch}", os.trim())
}

fn ensure_image(t: &Target, tag: &str) -> Result<String, String> {
    let machine = t.run("docker version --format '{{.Server.Os}} {{.Server.Arch}}'")?;
    let (os, arch) = machine.trim().split_once(' ').unwrap_or(("", ""));
    let machine = platform(os, arch);
    let there = t.run(&format!("docker image inspect --format '{{{{.Os}}}} {{{{.Architecture}}}}' {} 2>/dev/null || true", sq(tag)))?;
    let there = there.trim().split_once(' ').map(|(o, a)| platform(o, a)).unwrap_or_default();
    let here = Command::new("docker").args(["image", "inspect", "--format", "{{.Os}} {{.Architecture}}", tag]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let here = here.split_once(' ').map(|(o, a)| platform(o, a)).unwrap_or_default();
    let route = image_route(&machine, &there, &here, is_release_build());
    if route == ImageRoute::There {
        return Ok(format!("{:<8}{tag} (on {} already, {machine})", "image", t.host));
    }
    if route == ImageRoute::Send {
        super::progress(&format!("sending the image {tag} to {}", t.host));
        let mut save = Command::new("docker").args(["save", tag]).stdout(Stdio::piped()).spawn().map_err(|e| format!("docker save: {e}"))?;
        let stdout = save.stdout.take().ok_or("docker save: no output")?;
        let loaded = t.ssh(false).arg("docker load -q").stdin(stdout).stdout(Stdio::null()).status().map_err(|e| format!("ssh: {e}"))?;
        let saved = save.wait().map_err(|e| format!("docker save: {e}"))?;
        if !saved.success() || !loaded.success() {
            return Err(format!("the image {tag} could not be sent to {}", t.host));
        }
        return Ok(format!("{:<8}{tag} (sent from this machine, {machine})", "image"));
    }
    let mismatch = match &route {
        ImageRoute::Pull { mismatch: Some(p) } | ImageRoute::Build { mismatch: Some(p) } => format!(" ({tag} is {p} and {} runs {machine})", t.host),
        _ => String::new(),
    };
    if matches!(route, ImageRoute::Pull { .. }) {
        let published = format!("ghcr.io/hale-lang/hale:{}", env!("CARGO_PKG_VERSION"));
        super::progress(&format!("pulling {published} on {}{mismatch}", t.host));
        let ours = Command::new(std::env::current_exe().map_err(|e| e.to_string())?).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let theirs = t.run(&format!("docker pull -q --platform {m} {p} >/dev/null 2>&1 && docker run --rm --platform {m} --entrypoint hale {p} --version", p = sq(&published), m = sq(&machine))).map(|s| s.trim().to_string()).unwrap_or_default();
        if !theirs.is_empty() && theirs == ours {
            t.run(&format!("docker tag {} {}", sq(&published), sq(tag)))?;
            return Ok(format!("{:<8}{tag} (pulled there as {published})", "image"));
        }
        super::progress(&format!(
            "{published} {} on {}; building it there instead",
            if theirs.is_empty() { format!("has no {machine} image") } else { "is another build".to_string() },
            t.host
        ));
    }
    build_there(t, tag, &machine, &mismatch)
}

/// The toolchain image built on the machine from this toolchain's commit
/// on its repository, the source a local habitat's compose renders as its
/// `build:`. Its docker needs BuildKit (the `buildx` plugin): the
/// Dockerfile builds only the stages the chosen one needs.
fn build_there(t: &Target, tag: &str, machine: &str, mismatch: &str) -> Result<String, String> {
    let pin = HABITAT_PIN.trim();
    if pin.is_empty() {
        return Err(format!("no image of this toolchain reaches {}{mismatch}, and this toolchain was built outside a git checkout, so it names no commit to build one from (`--image <tag>` names an image there)", t.host));
    }
    let short = &pin[..12.min(pin.len())];
    if t.run("docker buildx version >/dev/null 2>&1 && echo yes || echo no")?.trim() != "yes" {
        return Err(format!(
            "building the toolchain image on {h} needs docker's buildx plugin there, which it lacks (Docker Desktop and Linux's docker packages ship it; Homebrew's docker does not). With Homebrew, once:\n  ssh {h} 'brew install docker-buildx && mkdir -p ~/.docker/cli-plugins && ln -sfn \"$(brew --prefix)/opt/docker-buildx/bin/docker-buildx\" ~/.docker/cli-plugins/docker-buildx'",
            h = t.host
        ));
    }
    super::progress(&format!("building the image {tag} on {} from hale-lang/hale@{short}{mismatch}; once per toolchain commit, and a first build compiles the compiler (tens of minutes)", t.host));
    let context = format!("{HALE_GIT}#{pin}");
    let built = t
        .ssh(false)
        .arg(format!("docker build -t {} {}", sq(tag), sq(&context)))
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::from(std::io::stderr()))
        .status()
        .map_err(|e| format!("ssh: {e}"))?;
    if !built.success() {
        return Err(format!("building {tag} on {} from {context} failed (above); the commit must be on GitHub, so push it, or `--image <tag>` names an image there", t.host));
    }
    Ok(format!("{:<8}{tag} (built on {} from hale-lang/hale@{short}, {machine})", "image", t.host))
}

/// The `docker run` on the machine that runs `hale habitat <args>` in the
/// toolchain image as its user, with its data directory and the docker
/// socket, on the host's network: the same `hale` a local habitat runs.
/// At a terminal (`tty`) the run is interactive with a terminal of its own.
fn in_image(m: &Machine, p: &serde_json::Value, args: &[&str], tty: bool) -> String {
    in_image_io(m, p, args, if tty { ImageIo::Terminal } else { ImageIo::None })
}

/// What a run in the toolchain image is given of this side's stdin: none,
/// the stream as it is (a pipe, `-i`), or a terminal (`-i -t`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum ImageIo {
    None,
    Stdin,
    Terminal,
}

fn in_image_io(m: &Machine, p: &serde_json::Value, args: &[&str], io: ImageIo) -> String {
    let data = data_dir(m);
    let image = p["image"].as_str().unwrap_or("");
    let mut env = vec![
        ("HOME", m.home.clone()),
        ("USER", m.user.clone()),
        ("XDG_DATA_HOME", format!("{}/.local/share", m.home)),
        // state a run there keeps (an organism head's launch token and
        // key) under the data directory every run there mounts
        ("XDG_STATE_HOME", format!("{data}/state")),
        ("HALE_DNA_HABITAT", habitat_dir(m)),
        ("HALE_VAULT_DIR", format!("{data}/vault")),
        ("HALE_HABITAT_IMAGE", image.to_string()),
        (LISTEN_ENV, p["listen"].as_str().unwrap_or("127.0.0.1").to_string()),
        (ADDRESS_ENV, p["host"].as_str().unwrap_or("").to_string()),
        (EXPECT_ENV, p["identity"].as_str().unwrap_or("").to_string()),
        // the machine's exporter is the person's login's to install over
        // SSH (a Mac's is launchd's, outside the image), never the image's
        ("HALE_DNA_NODE_EXPORTER", "off".to_string()),
    ];
    for k in FORWARDED {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                env.push((k, v));
            }
        }
    }
    // the socket's group as the run sees it: the daemon's socket, mounted
    // the same way into the same image
    let mut line = format!(
        "mkdir -p {d} && sg=$(docker run --rm --entrypoint stat -v /var/run/docker.sock:/var/run/docker.sock {i} -c %g /var/run/docker.sock) && exec docker run --rm {it}--network host --label {lb} --user {u}:{g} --group-add \"$sg\" -v {d}:{d} -v /var/run/docker.sock:/var/run/docker.sock -w {h}",
        d = sq(&data),
        i = sq(image),
        lb = sq(&format!("hale.habitat.dir={}", habitat_dir(m))),
        it = match io {
            ImageIo::Terminal => "-i -t ",
            ImageIo::Stdin => "-i ",
            ImageIo::None => "",
        },
        u = sq(&m.uid),
        g = sq(&m.gid),
        h = sq(&m.home),
    );
    for (k, v) in env {
        line.push_str(&format!(" -e {}", sq(&format!("{k}={v}"))));
    }
    line.push_str(&format!(" {} habitat", sq(image)));
    for a in args {
        line.push(' ');
        line.push_str(&sq(a));
    }
    line
}

/// The habitat's state on its machine the pointer is made from: the head's
/// URL, its launch token and the record's identity ("" for what is not
/// there).
fn remote_state(t: &Target, m: &Machine) -> Result<(String, String, String), String> {
    let hab = habitat_dir(m);
    let said = t.run(&format!(
        "cat {u} 2>/dev/null; echo '--'; cat {k} 2>/dev/null; echo '--'; cat {g} 2>/dev/null; true",
        u = sq(&format!("{hab}/head.url")),
        k = sq(&format!("{hab}/face-state/head.token")),
        g = sq(&format!("{hab}/genome/hale.toml")),
    ))?;
    let mut parts = said.splitn(3, "--\n");
    let url = parts.next().unwrap_or("").trim().to_string();
    let token = parts.next().unwrap_or("").trim().to_string();
    let identity = manifest_pin_in_text(parts.next().unwrap_or("")).unwrap_or_default();
    Ok((url, token, identity))
}

fn write_pointer(p: &serde_json::Value, token: &str) -> Result<(), String> {
    std::fs::create_dir_all(local_habitat_dir()).map_err(|e| format!("create {}: {e}", local_habitat_dir()))?;
    std::fs::write(head_url_file(), format!("{}\n", p["head"].as_str().unwrap_or(""))).map_err(|e| format!("write {}: {e}", head_url_file().display()))?;
    let state = face_state_dir();
    std::fs::create_dir_all(&state).map_err(|e| format!("create {}: {e}", state.display()))?;
    if !token.is_empty() {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = state.join("head.token");
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path).map_err(|e| format!("write {}: {e}", path.display()))?;
        f.write_all(format!("{token}\n").as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    std::fs::write(pointer_path(), format!("{}\n", serde_json::to_string_pretty(p).unwrap_or_default())).map_err(|e| format!("write {}: {e}", pointer_path().display()))
}

/// The pointer removed, last of what a forget does.
fn remove_pointer() {
    let _ = std::fs::remove_file(face_state_dir().join("head.token"));
    let _ = std::fs::remove_dir(face_state_dir());
    let _ = std::fs::remove_file(head_url_file());
    let _ = std::fs::remove_file(pointer_path());
    let _ = std::fs::remove_dir(local_habitat_dir());
}

/// Whether something accepts a connection at `url`'s host and port.
fn answers(url: &str) -> bool {
    use std::net::ToSocketAddrs;
    let rest = url.split("://").nth(1).unwrap_or(url);
    let hostport = rest.split('/').next().unwrap_or("");
    let Ok(addrs) = hostport.to_socket_addrs() else { return false };
    addrs.into_iter().any(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(500)).is_ok())
}

/// The face's URL beside the head's: the block's base, the port before it.
fn face_of(head: &str) -> String {
    match head.rsplit_once(':') {
        Some((h, p)) => match p.trim_end_matches('/').parse::<u16>() {
            Ok(port) if port > 0 => format!("{h}:{}", port - 1),
            _ => String::new(),
        },
        None => String::new(),
    }
}

/// `hale habitat up --host <name> [--user <u>] [--port <n>]`, with `up`'s
/// own flags (`rest`) passed on to the `up` run on the machine.
pub(super) fn up(host: &str, user: Option<&str>, port: Option<u16>, name: &str, image: Option<&str>, rest: &[String]) -> ExitCode {
    match up_lines(host, user, port, name, image, rest) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("hale habitat up --host {host}: {why}");
            ExitCode::from(1)
        }
    }
}

fn up_lines(host: &str, user: Option<&str>, port: Option<u16>, name: &str, image: Option<&str>, rest: &[String]) -> Result<Vec<String>, String> {
    let kept = pointer();
    if let Some(k) = &kept {
        if k["host"].as_str() != Some(host) || k["name"].as_str() != Some(name) {
            return Err(format!(
                "this machine points at the habitat `{}` on {} already: `hale habitat down --forget` forgets it first",
                k["name"].as_str().unwrap_or(""),
                k["host"].as_str().unwrap_or("")
            ));
        }
    } else if PathBuf::from(local_habitat_dir()).join("genome").exists() {
        return Err(format!("this machine runs a habitat of its own ({}): a machine points at one habitat, here or on another host", local_habitat_dir()));
    }
    let t = Target { host: host.to_string(), user: user.map(str::to_string), port };
    super::progress(&format!("reaching {host} over SSH"));
    let m = probe(&t)?;
    let listen = listen_on(&m.os, || listen_address(host))?;
    let tag = image.map(str::to_string).or_else(|| std::env::var("HALE_HABITAT_IMAGE").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())).unwrap_or_else(|| default_habitat_image_tag(HABITAT_PIN.trim()));
    // a second `up` reconnects: the habitat there must be the one the
    // pointer names, by its record's identity
    let (_, _, before) = remote_state(&t, &m)?;
    if let Some(k) = &kept {
        let recorded = k["identity"].as_str().unwrap_or("");
        if !recorded.is_empty() && before != recorded {
            return Err(format!(
                "the habitat on {host} is not the one this machine points at (its record is {}, the pointer's {}); nothing was changed",
                if before.is_empty() { "gone".to_string() } else { format!("{}…", &before[..12.min(before.len())]) },
                &recorded[..12.min(recorded.len())]
            ));
        }
    }
    let mut out = vec![format!("habitat {name} on {host} ({}, its user {})", habitat_dir(&m), m.user)];
    out.push(ensure_image(&t, &tag)?);
    let mut p = serde_json::json!({
        "name": name,
        "host": host,
        "user": user.unwrap_or(""),
        "ssh_port": port.unwrap_or(0),
        "listen": listen,
        "image": tag,
        "dir": habitat_dir(&m),
    });
    let mut args: Vec<&str> = vec!["up", "--name", name];
    args.extend(rest.iter().map(String::as_str));
    super::progress(&format!("bringing the habitat up on {host}"));
    let ran = t.ssh(false).arg(in_image(&m, &p, &args, false)).stdin(Stdio::null()).status().map_err(|e| format!("ssh: {e}"))?;
    if !ran.success() {
        return Err(format!("`hale habitat up` on {host} failed (exit {}); nothing here points at it", ran.code().unwrap_or(-1)));
    }
    let (url, token, identity) = remote_state(&t, &m)?;
    if url.is_empty() || identity.is_empty() {
        return Err(format!("the habitat on {host} came up without a head URL or a record identity; nothing here points at it"));
    }
    if !before.is_empty() && before != identity {
        return Err(format!("the habitat on {host} changed its record identity while it came up; nothing here points at it"));
    }
    // the same readiness a local `up` waits on, over the recorded URL
    let face = face_of(&url);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !(answers(&url) && answers(&face)) {
        if std::time::Instant::now() > deadline {
            return Err(format!("the habitat's head and face did not answer here on {url} and {face} within 180s"));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    p["head"] = serde_json::json!(url);
    p["face"] = serde_json::json!(face);
    p["identity"] = serde_json::json!(identity);
    write_pointer(&p, &token)?;
    out.push(format!("{:<8}{face}/", "face"));
    out.push(format!("{:<8}{url} (recorded here in {})", "head", head_url_file().display()));
    let link = super::sign_in_link(&face);
    if !link.is_empty() {
        out.push(format!("{:<8}{link}", "sign in"));
    }
    out.push(format!("{:<8}{}… ({})", "record", &identity[..12.min(identity.len())], if kept.is_some() { "reconnected; the pointer is the same habitat's" } else { "pointed at from this machine" }));
    if !super::readings::grafana_off() {
        out.push(format!("{:<8}{face}{}", "grafana", super::readings::GRAFANA_PATH));
    }
    if !super::readings::node_exporter_off() {
        super::progress(&format!("installing the node and container exporters on {host}, over your SSH login"));
        out.extend(super::readings::install_over(host, user, port));
    }
    Ok(out)
}

/// `hale habitat status` of a habitat on another machine: the pointer, and
/// what its head answers (`Commands::index`, `Commands::allocations`).
pub(super) fn status(p: &serde_json::Value) -> ExitCode {
    let head = p["head"].as_str().unwrap_or("");
    println!("habitat {} on {} ({}; this machine holds its pointer, {})", p["name"].as_str().unwrap_or(""), p["host"].as_str().unwrap_or(""), p["dir"].as_str().unwrap_or(""), pointer_path().display());
    println!("{:<8}{}", "name", p["name"].as_str().unwrap_or(""));
    let identity = p["identity"].as_str().unwrap_or("");
    println!("{:<8}on {} (record {}…)", "genome", p["host"].as_str().unwrap_or(""), &identity[..12.min(identity.len())]);
    let up = answers(head);
    let face_up = answers(p["face"].as_str().unwrap_or(""));
    println!("{:<8}{}/ ({})", "face", p["face"].as_str().unwrap_or(""), if face_up { "up" } else { "down: `hale habitat up --host` starts it" });
    let link = super::sign_in_link(p["face"].as_str().unwrap_or(""));
    if face_up && !link.is_empty() {
        println!("{:<8}{link}", "sign in");
    }
    if face_up && !super::readings::grafana_off() {
        println!("{:<8}{}{}", "grafana", p["face"].as_str().unwrap_or(""), super::readings::GRAFANA_PATH);
    }
    println!("{:<8}{head} ({})", "head", if up { "up" } else { "down" });
    if !up {
        return ExitCode::SUCCESS;
    }
    // the organisms by their names: the habitat's own by the pointer's, each
    // grafted one by the index's (an allocation names its record's identity)
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    names.insert(identity[..12.min(identity.len())].to_string(), p["name"].as_str().unwrap_or("").to_string());
    match http_surface_call(head, "Commands::index", &serde_json::json!({ "limit": 200 })) {
        Ok(v) => {
            if v["ok"] == serde_json::json!(true) && v["name"].as_str().is_some_and(|n| n != p["name"].as_str().unwrap_or("")) {
                println!("{:<8}the head answers as the habitat `{}`, not this pointer's", "note", v["name"].as_str().unwrap_or(""));
            }
            let organisms: Vec<serde_json::Value> = v["organisms"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
            for o in organisms {
                if let (Some(k), Some(n)) = (o["key"].as_str(), o["name"].as_str()) {
                    names.insert(k[..12.min(k.len())].to_string(), n.to_string());
                }
            }
        }
        Err(e) => println!("{:<8}{e}", "note"),
    }
    match http_surface_call(head, "Commands::allocations", &serde_json::json!({ "limit": 200 })) {
        Ok(v) if v["ok"] == serde_json::json!(true) => {
            let rows: Vec<serde_json::Value> = v["allocations"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
            println!("allocations, as the head answers:");
            if rows.is_empty() {
                println!("allocations: none");
            }
            for r in rows {
                let f = |k: &str| r[k].as_str().unwrap_or("").to_string();
                // the habitat's own machine is the host `local` in its rows; here it is the machine's name
                let on = if f("applied").is_empty() { f("host") } else { f("applied") };
                let on = if on == "local" { p["host"].as_str().unwrap_or("local").to_string() } else { on };
                let org = f("organism");
                let org = names.get(&org[..12.min(org.len())]).cloned().unwrap_or(org);
                // how it runs, and what fills it when that is another thing
                let by = if f("runs_by").is_empty() || f("runs_by") == f("provider") { f("provider") } else { format!("{} ({})", f("runs_by"), f("provider")) };
                println!("  {:<24} {:<8} {} on {}{}: {}{}", f("allocation"), f("role"), by, on, if org.is_empty() { String::new() } else { format!(", {org}") }, f("state"), if f("detail").is_empty() { String::new() } else { format!(" ({})", f("detail")) });
            }
            ExitCode::SUCCESS
        }
        Ok(v) => {
            eprintln!("hale habitat status: the head answered no allocations ({})", v["why"].as_str().unwrap_or(""));
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("hale habitat status: {e}");
            ExitCode::from(1)
        }
    }
}

/// `hale habitat down [--forget [--plan]]` of a habitat on another machine:
/// run there over SSH. `--forget` is planned and confirmed there, by the
/// habitat's name typed at the person's terminal (SSH carries it); the
/// pointer here is removed last, once the habitat there is forgotten.
pub(super) fn down(p: &serde_json::Value, args: &[String]) -> ExitCode {
    let forget = args.iter().any(|a| a == "--forget");
    let plan = args.iter().any(|a| a == "--plan");
    let valid = matches!(args.len(), 0) || (forget && args.len() <= 2 && args.iter().all(|a| a == "--forget" || a == "--plan"));
    if !valid {
        eprintln!("usage: hale habitat down [--forget [--plan]]");
        return ExitCode::from(2);
    }
    let confirm = forget && !plan;
    if confirm && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!("hale habitat down --forget: stdin is not a terminal; forgetting the habitat removes its record, its organisms and their data, and is confirmed only by typing the habitat's name at a terminal (`--plan` prints the plan)");
        return ExitCode::from(2);
    }
    let t = Target::of(p);
    let m = match probe(&t) {
        Ok(m) => m,
        Err(why) => {
            eprintln!("hale habitat down: {why}");
            return ExitCode::from(1);
        }
    };
    // the habitat there must be the one this pointer names, by its
    // record's identity, before anything there is stopped or removed; the
    // run there checks it again (`HALE_DNA_HABITAT_EXPECT`), so a habitat
    // made there in between is never what it acts on
    let expected = p["identity"].as_str().unwrap_or("").to_string();
    let found = match remote_state(&t, &m) {
        Ok((_, _, identity)) => identity,
        Err(why) => {
            eprintln!("hale habitat down: {why}");
            return ExitCode::from(1);
        }
    };
    if let Some(why) = pointer_refusal(&t.host, &expected, &found) {
        eprintln!("hale habitat down: {why}");
        return ExitCode::from(1);
    }
    // the hosts the habitat placed on, read before it is forgotten: its key
    // comes off each over the person's own login once the forget is done
    let hosts: Vec<serde_json::Value> = if forget {
        http_surface_call(p["head"].as_str().unwrap_or(""), "Commands::hosts", &serde_json::json!({ "limit": 200 }))
            .ok()
            .and_then(|v| v["hosts"].as_str().and_then(|t| serde_json::from_str::<Vec<serde_json::Value>>(t).ok()))
            .unwrap_or_default()
            .into_iter()
            .filter(|h| h["name"].as_str().is_some_and(|n| !n.is_empty() && n != "local") && h["state"] != "removed")
            .collect()
    } else {
        Vec::new()
    };
    let comment = key_comment(&expected);
    let mut a: Vec<&str> = vec!["down"];
    a.extend(args.iter().map(String::as_str));
    let mut cmd = t.ssh(confirm);
    cmd.arg(in_image(&m, p, &a, confirm));
    if !confirm {
        cmd.stdin(Stdio::null());
    }
    let ran = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hale habitat down: ssh: {e}");
            return ExitCode::from(1);
        }
    };
    if !ran.success() {
        return ExitCode::from(ran.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1));
    }
    if plan {
        if !super::readings::node_exporter_off() {
            println!("  - and from this machine, over your SSH login: the habitat's node exporter off {} and off each host below, where it installed one", t.host);
        }
        for h in &hosts {
            let user = h["user"].as_str().unwrap_or("");
            if is_habitat_user(user) {
                println!("  - and from this machine, over your SSH login: the habitat's account {user} and its home off {}", h["name"].as_str().unwrap_or(""));
            } else {
                println!("  - and from this machine, over your SSH login: the habitat's key (`{comment}`) out of {user}@{}:~/.ssh/authorized_keys", h["name"].as_str().unwrap_or(""));
            }
        }
    }
    if confirm {
        // gone there whole: the pointer goes last
        match remote_state(&t, &m) {
            Ok((_, _, identity)) if identity.is_empty() => {
                for h in &hosts {
                    let (name, user) = (h["name"].as_str().unwrap_or(""), h["user"].as_str().unwrap_or(""));
                    let port = h["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
                    if is_habitat_user(user) {
                        match remove_user(name, port, user) {
                            Ok(()) => println!("the habitat's account {user} is removed from {name}"),
                            Err(why) => println!("the habitat's account {user} is left on {name} ({why}): remove it there (Linux `sudo userdel -r {user}`, macOS `sudo dscl . -delete /Users/{user}`)"),
                        }
                        continue;
                    }
                    match unauthorize_key(name, user, port, &comment) {
                        Ok(()) => println!("the habitat's key is out of {user}@{name}:~/.ssh/authorized_keys"),
                        Err(why) => println!("the habitat's key is left on {name} ({why}): remove the line ending `{comment}` from {user}'s ~/.ssh/authorized_keys there"),
                    }
                }
                if !super::readings::node_exporter_off() {
                    let machines = std::iter::once((t.host.clone(), t.user.clone(), t.port)).chain(hosts.iter().map(|h| {
                        (h["name"].as_str().unwrap_or("").to_string(), None, h["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0))
                    }));
                    for (name, user, port) in machines {
                        for l in super::readings::remove_over(&name, user.as_deref(), port) {
                            println!("{l}");
                        }
                    }
                }
                remove_pointer();
                println!("the pointer to the habitat on {} is removed from this machine", t.host);
            }
            Ok(_) => {
                eprintln!("hale habitat down --forget: the habitat on {} still holds its record; this machine's pointer is kept", t.host);
                return ExitCode::from(1);
            }
            Err(why) => {
                eprintln!("hale habitat down --forget: {why}; this machine's pointer is kept");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

/// Why the habitat on `host` must not be stopped or removed through a
/// pointer to the record `expected` when the record there is `found`
/// ("" for none): another record there, or none.
fn pointer_refusal(host: &str, expected: &str, found: &str) -> Option<String> {
    let short = |id: &str| format!("{}…", &id[..12.min(id.len())]);
    if expected.is_empty() {
        return Some(format!("this machine's pointer names no record of the habitat on {host}; nothing there is touched (`hale habitat up --host {host}` reconnects it)"));
    }
    if found.is_empty() {
        return Some(format!("the habitat on {host} holds no record (the pointer's is {}): nothing there is touched; remove {} to forget the pointer", short(expected), pointer_path().display()));
    }
    if found != expected {
        return Some(format!("the habitat on {host} is not the one this machine points at (its record is {}, the pointer's {}); nothing there is touched", short(found), short(expected)));
    }
    None
}

/// The machine and its record checked against the pointer before an act
/// there: the probe, or why the act is refused (the exit code beside it).
fn checked_machine(p: &serde_json::Value, verb: &str) -> Result<(Target, Machine), ExitCode> {
    let t = Target::of(p);
    let m = probe(&t).map_err(|why| {
        eprintln!("hale habitat {verb}: {why}");
        ExitCode::from(1)
    })?;
    let expected = p["identity"].as_str().unwrap_or("").to_string();
    let found = remote_state(&t, &m).map(|(_, _, id)| id).map_err(|why| {
        eprintln!("hale habitat {verb}: {why}");
        ExitCode::from(1)
    })?;
    if let Some(why) = pointer_refusal(&t.host, &expected, &found) {
        eprintln!("hale habitat {verb}: {why}");
        return Err(ExitCode::from(1));
    }
    Ok((t, m))
}

/// Where a directory grafted through a pointer lives on the habitat's
/// machine: the habitat's own copy, `<habitat>/grafted/<name>`, which the
/// organism's removal and the habitat's forget delete.
fn grafted_dir(m: &Machine, name: &str) -> String {
    format!("{}/grafted/{name}", habitat_dir(m))
}

/// `hale habitat graft <dir> [flags]` through a pointer: the graft runs on
/// the habitat's machine, as `up` and `down` do. A directory that exists
/// here is shipped there whole (`tar | ssh`) as the habitat's copy and
/// becomes the organism's home; a path that does not exist is a new
/// application made there. Every flag is passed through, and the output
/// streams back. A graft that fails leaves no copy it shipped.
pub(super) fn graft(p: &serde_json::Value, args: &[String]) -> ExitCode {
    let Some(given) = args.first() else {
        eprintln!("usage: hale habitat graft <dir> [flags]");
        return ExitCode::from(2);
    };
    let local = PathBuf::from(given);
    let name = match std::fs::canonicalize(&local).ok().as_deref().unwrap_or(&local).file_name().and_then(|n| n.to_str()) {
        Some(n) if !n.is_empty() && n != "." && n != ".." => n.to_string(),
        _ => {
            eprintln!("hale habitat graft: `{given}` names no directory");
            return ExitCode::from(2);
        }
    };
    let (t, m) = match checked_machine(p, "graft") {
        Ok(tm) => tm,
        Err(code) => return code,
    };
    // a directory of its own for this graft, made fresh there: `mkdir`
    // (never `-p`) is the gate, so nothing is ever unpacked over another
    // organism's home, and the organism's directory keeps its name inside it
    let gate = grafted_dir(&m, &format!("{name}-{}", fresh_suffix()));
    let there = format!("{gate}/{name}");
    let made = t.run(&format!("mkdir -p {g} && mkdir {d} && echo made || true", g = sq(&format!("{}/grafted", habitat_dir(&m))), d = sq(&gate)));
    if made.as_deref().map(str::trim) != Ok("made") {
        eprintln!("hale habitat graft: {gate} could not be made fresh on {}; nothing was shipped", t.host);
        return ExitCode::from(1);
    }
    let shipped = local.is_dir();
    if shipped {
        super::progress(&format!("shipping {} to {}:{there}", local.display(), t.host));
        let Some(parent) = std::fs::canonicalize(&local).ok().and_then(|c| c.parent().map(|p| p.to_path_buf())) else {
            eprintln!("hale habitat graft: `{given}` has no parent directory");
            let _ = t.run(&format!("rmdir {}", sq(&gate)));
            return ExitCode::from(2);
        };
        let mut tar = match Command::new("tar").arg("-C").arg(&parent).args(["-cf", "-", &name]).stdout(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("hale habitat graft: tar: {e}");
                let _ = t.run(&format!("rmdir {}", sq(&gate)));
                return ExitCode::from(1);
            }
        };
        let Some(out) = tar.stdout.take() else { return ExitCode::from(1) };
        let unpacked = t.ssh(false).arg(format!("tar -C {} -xf -", sq(&gate))).stdin(out).status();
        let packed = tar.wait();
        if !unpacked.is_ok_and(|s| s.success()) || !packed.is_ok_and(|s| s.success()) {
            // nothing grafted from it yet: the fresh directory goes whole
            eprintln!("hale habitat graft: {} could not be shipped to {}; nothing was grafted", local.display(), t.host);
            let _ = t.run(&format!("rm -rf {}", sq(&gate)));
            return ExitCode::from(1);
        }
        println!("{:<8}{} shipped to {}:{there}, the habitat's copy", "shipped", local.display(), t.host);
    }
    // what the directory held before the graft: a pin or DNA state it
    // brought makes it never removable here
    let before = graft_state(&t, &there);
    let mut a: Vec<&str> = vec!["graft", there.as_str()];
    a.extend(args[1..].iter().map(String::as_str));
    let ran = t.ssh(false).arg(in_image(&m, p, &a, false)).stdin(Stdio::null()).status();
    if ran.as_ref().is_ok_and(|s| s.success()) {
        return ExitCode::SUCCESS;
    }
    // a refusal that committed nothing leaves no copy; anything else (a
    // pin or DNA state the graft wrote, a result SSH lost) keeps it, since
    // the graft resumes from it
    let code = ran.as_ref().ok().and_then(|s| s.code());
    let after = graft_state(&t, &there);
    if keep_after_failure(code, before.as_deref(), after.as_deref()) {
        eprintln!("hale habitat graft: the graft on {} did not finish; its directory is kept at {there}; go on from it with:\n  {}", t.host, resume_command(&there, &args[1..]));
    } else {
        let _ = t.run(&format!("rm -rf {}", sq(&gate)));
        eprintln!("hale habitat graft: the graft on {} was refused before it committed anything; the copy made for it there is removed", t.host);
    }
    ExitCode::from(code.and_then(|c| u8::try_from(c).ok()).unwrap_or(1))
}

/// The command that goes on with a graft whose directory is kept at
/// `there` on the habitat's machine, with the same flags.
fn resume_command(there: &str, flags: &[String]) -> String {
    let mut line = format!("hale habitat graft --resume {}", sq(there));
    for f in flags {
        line.push(' ');
        line.push_str(&if f.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:@,".contains(c)) { f.clone() } else { sq(f) });
    }
    line
}

/// The flags a resumed graft runs with: always attached to the kept
/// directory, which a graft that stopped already holds (a creation graft's
/// new application among them, whose pin it goes on from), and the rest as
/// given.
fn resume_flags(flags: &[String]) -> Vec<String> {
    let mut out = vec!["--attach".to_string()];
    out.extend(flags.iter().filter(|f| f.as_str() != "--attach").cloned());
    out
}

/// Why `path` is not a graft's directory kept on the habitat's machine
/// whose habitat lives at `habitat`: one under `<habitat>/grafted/`, named
/// without `.` or `..`.
fn resume_refusal(habitat: &str, path: &str) -> Option<String> {
    let base = format!("{}/grafted/", habitat.trim_end_matches('/'));
    let Some(rest) = path.strip_prefix(&base) else {
        return Some(format!("`{path}` is not a graft's directory on the habitat's machine (those are under {base})"));
    };
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    if parts.is_empty() || parts.iter().any(|c| c.is_empty() || *c == "." || *c == "..") {
        return Some(format!("`{path}` names no directory under {base}"));
    }
    None
}

/// `hale habitat graft --resume <path> [flags]` through a pointer: the
/// graft goes on in a directory a graft through the pointer made and kept
/// on the habitat's machine, with nothing shipped: the path must be under
/// `<habitat>/grafted/` and exist there, after the pointer's record is
/// checked.
pub(super) fn graft_resume(p: &serde_json::Value, args: &[String]) -> ExitCode {
    let Some(path) = args.first().filter(|a| !a.starts_with('-')) else {
        eprintln!("usage: hale habitat graft --resume <path on the habitat's machine> [flags]");
        return ExitCode::from(2);
    };
    let (t, m) = match checked_machine(p, "graft") {
        Ok(tm) => tm,
        Err(code) => return code,
    };
    if let Some(why) = resume_refusal(&habitat_dir(&m), path) {
        eprintln!("hale habitat graft --resume: {why}");
        return ExitCode::from(2);
    }
    if t.run(&format!("test -d {} && echo yes || true", sq(path))).map(|o| o.trim() == "yes") != Ok(true) {
        eprintln!("hale habitat graft --resume: {path} does not exist on {}", t.host);
        return ExitCode::from(1);
    }
    let flags = resume_flags(&args[1..]);
    let mut a: Vec<&str> = vec!["graft", path.as_str()];
    a.extend(flags.iter().map(String::as_str));
    let ran = t.ssh(false).arg(in_image(&m, p, &a, false)).stdin(Stdio::null()).status();
    if ran.as_ref().is_ok_and(|s| s.success()) {
        return ExitCode::SUCCESS;
    }
    eprintln!("hale habitat graft: the graft on {} did not finish; its directory is kept at {path}; go on from it with:\n  {}", t.host, resume_command(path, &args[1..]));
    ExitCode::from(ran.ok().and_then(|s| s.code()).and_then(|c| u8::try_from(c).ok()).unwrap_or(1))
}

/// 12 hex digits drawn fresh: the suffix of a graft's own directory.
fn fresh_suffix() -> String {
    let mut b = [0u8; 6];
    let _ = openssl::rand::rand_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What a graft has committed in the directory `there` on the machine: a
/// line naming each mark present (`pin`: its hale.toml pins a record;
/// `dna`: `.hale/dna` exists), `None` when the machine did not answer.
fn graft_state(t: &Target, there: &str) -> Option<String> {
    t.run(&format!(
        "d={}; s=''; grep -qs '^record *=' \"$d/hale.toml\" && s=\"$s pin\"; [ -e \"$d/.hale/dna\" ] && s=\"$s dna\"; echo \"state:$s\"",
        sq(there)
    ))
    .ok()
    .and_then(|o| o.lines().find_map(|l| l.strip_prefix("state:").map(|s| s.trim().to_string())))
}

/// Whether a failed graft's directory must be kept: unless the graft
/// answered (a code SSH carried back, not 255) and the directory holds no
/// mark of a graft, now or before it ran.
fn keep_after_failure(code: Option<i32>, before: Option<&str>, after: Option<&str>) -> bool {
    match (code, before, after) {
        (Some(c), Some(b), Some(a)) if c != 255 => !b.is_empty() || !a.is_empty(),
        _ => true,
    }
}

/// The habitat's vault on its machine, asked over the person's own login
/// after the pointer's record is checked there: `script` (made for the
/// vault's directory there, `hosted::vault_script`) run by the login's
/// shell, and what it printed. A value it prints is held in this process's
/// memory alone, never written here.
pub(super) fn vault_on_machine(p: &serde_json::Value, script: impl Fn(&str) -> String) -> Result<String, String> {
    let t = Target::of(p);
    let m = probe(&t)?;
    let expected = p["identity"].as_str().unwrap_or("").to_string();
    let found = remote_state(&t, &m).map(|(_, _, id)| id)?;
    if let Some(why) = pointer_refusal(&t.host, &expected, &found) {
        return Err(why);
    }
    t.run(&script(&format!("{}/vault", data_dir(&m))))
}

/// `hale habitat organism <name> dna <verb> [args…]` through a pointer:
/// the organism's own verbs (`review`, `task create`, `work run`, …) run in
/// its directory on the habitat's machine, at this terminal when there is
/// one. Stdin is forwarded either way, so a verb that reads it (`secret
/// set`, its value piped in) gets what was piped here; off a terminal no
/// terminal is allocated, there or on the ssh connection.
pub(super) fn organism_dna(p: &serde_json::Value, args: &[String]) -> ExitCode {
    let (t, m) = match checked_machine(p, "organism") {
        Ok(tm) => tm,
        Err(code) => return code,
    };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut a: Vec<&str> = vec!["organism"];
    a.extend(args.iter().map(String::as_str));
    let mut cmd = t.ssh(tty);
    cmd.arg(in_image_io(&m, p, &a, organism_io(tty)));
    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat organism: ssh: {e}");
            ExitCode::from(1)
        }
    }
}

/// A habitat verb run on the habitat's machine, in its toolchain image, as
/// `organism … dna` is: for the verbs that act on the genome there
/// (`nursery refresh`).
pub(super) fn run_there(p: &serde_json::Value, verb: &str, args: &[&str]) -> ExitCode {
    let (t, m) = match checked_machine(p, verb) {
        Ok(tm) => tm,
        Err(code) => return code,
    };
    let mut cmd = t.ssh(false);
    cmd.arg(in_image(&m, p, args, false)).stdin(Stdio::null());
    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat {verb}: ssh: {e}");
            ExitCode::from(1)
        }
    }
}

/// `hale habitat graft <sapling> [flags]` through a pointer: the graft run on
/// the habitat's machine in its toolchain image, as `graft <dir>` is, with
/// every flag passed on (`--host`, `--own`, `--reuse`, `--plan`, …): its plan
/// and its progress print here as they happen, and at a terminal the person
/// confirms the plan there. `--no-head`: it acts itself, never through the
/// head, which would take the plan as it stands and ask no one.
pub(super) fn graft_sapling(p: &serde_json::Value, args: &[String]) -> ExitCode {
    let (t, m) = match checked_machine(p, "graft") {
        Ok(tm) => tm,
        Err(code) => return code,
    };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut a: Vec<&str> = vec!["graft"];
    a.extend(args.iter().map(String::as_str));
    a.push("--no-head");
    let mut cmd = t.ssh(tty);
    cmd.arg(in_image(&m, p, &a, tty));
    if !tty {
        cmd.stdin(Stdio::null());
    }
    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat graft: ssh: {e}");
            ExitCode::from(1)
        }
    }
}

/// What an organism's verb through the pointer is given of stdin: a
/// terminal at one, the stream as it is otherwise (never none: a value piped
/// to `secret set` has to arrive).
fn organism_io(tty: bool) -> ImageIo {
    if tty {
        ImageIo::Terminal
    } else {
        ImageIo::Stdin
    }
}

/// `up` asked without `--host` of a machine that points at a habitat on
/// another.
pub(super) fn refuse_local_up(p: &serde_json::Value) -> ExitCode {
    eprintln!(
        "hale habitat up: this machine points at the habitat `{}` on {}: `hale habitat up --host {}` reconnects it\n{}",
        p["name"].as_str().unwrap_or(""),
        p["host"].as_str().unwrap_or(""),
        p["host"].as_str().unwrap_or(""),
        usage_text().lines().next().unwrap_or("")
    );
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_of_another_platform_is_never_used_or_sent() {
        assert_eq!(image_route("linux/amd64", "linux/amd64", "", false), ImageRoute::There);
        assert_eq!(image_route("linux/amd64", "", "linux/amd64", false), ImageRoute::Send);
        assert_eq!(image_route("linux/amd64", "", "linux/arm64", true), ImageRoute::Pull { mismatch: Some("linux/arm64".into()) });
        assert_eq!(image_route("linux/amd64", "linux/arm64", "linux/amd64", true), ImageRoute::Send);
        assert_eq!(image_route("linux/arm64", "", "", true), ImageRoute::Pull { mismatch: None });
        assert_eq!(platform("linux", "x86_64"), "linux/amd64");
        assert_eq!(platform("linux", "aarch64"), "linux/arm64");
    }

    #[test]
    fn a_build_from_another_commit_builds_its_image_there_and_never_pulls_the_release() {
        assert_eq!(image_route("linux/arm64", "", "", false), ImageRoute::Build { mismatch: None });
        assert_eq!(image_route("linux/arm64", "", "linux/amd64", false), ImageRoute::Build { mismatch: Some("linux/amd64".into()) });
        assert_eq!(image_route("linux/arm64", "linux/arm64", "", false), ImageRoute::There);
    }

    #[test]
    fn a_release_is_the_commit_its_tag_names() {
        let annotated = "1111111111111111111111111111111111111111\trefs/tags/v0.29.0\n2222222222222222222222222222222222222222\trefs/tags/v0.29.0^{}\n";
        assert_eq!(tag_commit(annotated, "v0.29.0").as_deref(), Some("2222222222222222222222222222222222222222"));
        let light = "3333333333333333333333333333333333333333\trefs/tags/v0.29.0\n";
        assert_eq!(tag_commit(light, "v0.29.0").as_deref(), Some("3333333333333333333333333333333333333333"));
        assert_eq!(tag_commit("", "v0.29.0"), None);
        assert_eq!(tag_commit("4444\trefs/tags/v0.29.1\n", "v0.29.0"), None);
    }

    #[test]
    fn a_habitat_is_torn_down_only_through_its_own_pointer() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        assert_eq!(pointer_refusal("rack-a.lan", &a, &a), None);
        assert!(pointer_refusal("rack-a.lan", &a, &b).is_some_and(|w| w.contains("is not the one this machine points at")));
        assert!(pointer_refusal("rack-a.lan", &a, "").is_some_and(|w| w.contains("holds no record")));
        assert!(pointer_refusal("rack-a.lan", "", &b).is_some());
    }

    #[test]
    fn a_resumed_graft_attaches_to_the_kept_directory() {
        let s = |v: &[&str]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };
        // a creation graft's flags: attached on resume
        assert_eq!(resume_flags(&s(&["--no-library", "--no-pack"])), s(&["--attach", "--no-library", "--no-pack"]));
        // an attach graft's: once, not twice
        assert_eq!(resume_flags(&s(&["--attach", "--with", "founder"])), s(&["--attach", "--with", "founder"]));
        assert_eq!(resume_flags(&[]), s(&["--attach"]));
    }

    #[test]
    fn a_graft_resumes_only_in_a_kept_directory_of_its_habitat() {
        let h = "/home/h/.local/share/hale/habitat/local";
        assert_eq!(resume_refusal(h, &format!("{h}/grafted/shipme-0123456789ab/shipme")), None);
        assert!(resume_refusal(h, "/home/h/elsewhere/shipme").is_some());
        assert!(resume_refusal(h, &format!("{h}/grafted/")).is_some());
        assert!(resume_refusal(h, &format!("{h}/grafted/../genome")).is_some());
        assert!(resume_refusal(h, &format!("{h}/grafted/shipme-x/./shipme")).is_some());
        assert!(resume_refusal(h, &format!("{h}/genome")).is_some());
        assert_eq!(
            resume_command(&format!("{h}/grafted/a-1/a"), &["--attach".to_string(), "--set".to_string(), "who=a b".to_string()]),
            format!("hale habitat graft --resume '{h}/grafted/a-1/a' --attach --set 'who=a b'")
        );
    }

    #[test]
    fn a_failed_grafts_directory_goes_only_when_nothing_was_committed() {
        assert!(!keep_after_failure(Some(1), Some(""), Some("")));
        assert!(keep_after_failure(Some(1), Some(""), Some("pin dna")));
        assert!(keep_after_failure(Some(1), Some("pin"), Some("pin")));
        assert!(keep_after_failure(Some(255), Some(""), Some("")));
        assert!(keep_after_failure(None, Some(""), Some("")));
        assert!(keep_after_failure(Some(1), Some(""), None));
    }

    #[test]
    fn a_vm_docker_listens_on_the_vm_and_a_linux_docker_on_the_address() {
        assert_eq!(listen_on("Darwin", || Err("unused".into())).unwrap(), "0.0.0.0");
        assert_eq!(listen_on("Linux", || Ok("192.168.1.20".into())).unwrap(), "192.168.1.20");
        assert!(listen_on("Linux", || Err("rack-a.lan does not resolve here".into())).is_err());
    }

    /// The habitat's key goes into a host's authorized_keys once, and comes
    /// out by its comment alone: every other line stays as it was.
    #[test]
    fn the_habitats_key_is_put_on_a_host_once_and_taken_off_alone() {
        let home = std::env::temp_dir().join(format!("hale_authorize_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let keys = home.join(".ssh/authorized_keys");
        std::fs::write(&keys, "ssh-ed25519 AAAAperson person@laptop\n").unwrap();
        let run = |script: &str| assert!(Command::new("sh").arg("-c").arg(script).env("HOME", &home).status().unwrap().success(), "{script}");
        let comment = key_comment("0123456789abcdef");
        assert_eq!(comment, "habitat-0123456789ab");
        let line = format!("ssh-ed25519 AAAAhabitat {comment}");
        run(&authorize_script(&line));
        run(&authorize_script(&line));
        assert_eq!(std::fs::read_to_string(&keys).unwrap(), format!("ssh-ed25519 AAAAperson person@laptop\n{line}\n"), "appended once");
        run(&unauthorize_script(&comment));
        assert_eq!(std::fs::read_to_string(&keys).unwrap(), "ssh-ed25519 AAAAperson person@laptop\n", "the habitat's line alone removed");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A habitat's own account is named from its record and is the only
    /// account teardown deletes: any other name, the person's included, is
    /// never one.
    #[test]
    fn only_the_habitats_own_account_is_ever_removed() {
        let u = habitat_user("0123456789abcdef0123");
        assert_eq!(u, "hale-0123456789ab");
        assert!(is_habitat_user(&u));
        for other in ["person", "hale", "hale-", "hale-0123456789a", "hale-0123456789abc", "hale-0123456789ag", "root"] {
            assert!(!is_habitat_user(other), "{other}");
        }
        assert!(remove_user("box-1", None, "person").is_err(), "refused before any connection");
        let removal = remove_user_script(&u, "sudo -n");
        assert!(removal.contains("loginctl terminate-user") && removal.contains("pkill -KILL -u") && removal.contains("userdel -r"), "{removal}");
        assert!(Command::new("sh").arg("-n").arg("-c").arg(&removal).status().unwrap().success(), "the removal parses as sh");
        let script = provision_script(&u, "ssh-ed25519 AAAA habitat-0123456789ab", false);
        assert!(script.contains("u='hale-0123456789ab'") && script.contains("'ssh-ed25519 AAAA habitat-0123456789ab'") && script.contains("S=\"sudo -n\"") && script.contains("$S useradd --system"), "{script}");
        // a Mac's account: hidden, below uid 500, no password, admitted to Remote Login
        assert!(script.contains("dscl . -create \"/Users/$u\" IsHidden 1") && script.contains("Password '*'") && script.contains("com.apple.access_ssh"), "{script}");
        assert!(removal.contains("dscl . -delete \"/Users/$u\""), "{removal}");
        for tty in [false, true] {
            let parsed = Command::new("sh").arg("-n").arg("-c").arg(provision_script(&u, "ssh-ed25519 AAAA it's habitat-0123456789ab", tty)).status().unwrap();
            assert!(parsed.success(), "the account script parses as sh");
        }
    }

    #[test]
    fn a_remote_word_is_quoted_whole() {
        assert_eq!(sq("a b"), "'a b'");
        assert_eq!(sq("it's"), "'it'\\''s'");
    }

    #[test]
    fn the_face_is_the_port_before_the_head() {
        assert_eq!(face_of("http://rack-a.lan:8801"), "http://rack-a.lan:8800");
        assert_eq!(face_of("http://rack-a.lan"), "");
    }

    #[test]
    fn an_organisms_verb_gets_stdin_and_a_terminal_only_at_one() {
        let m = Machine { home: "/home/h".into(), uid: "1000".into(), gid: "1000".into(), user: "h".into(), os: "Linux".into() };
        let p = serde_json::json!({ "image": "hale-habitat:abc" });
        let piped = in_image_io(&m, &p, &["organism", "x", "dna", "secret", "set", "K"], organism_io(false));
        assert!(piped.contains("docker run --rm -i --network host"), "{piped}");
        let at_terminal = in_image_io(&m, &p, &["organism", "x", "dna", "status"], organism_io(true));
        assert!(at_terminal.contains("docker run --rm -i -t --network host"), "{at_terminal}");
        assert!(!in_image(&m, &p, &["up"], false).contains(" -i "));
    }

    #[test]
    fn the_run_there_carries_its_address_and_its_data() {
        let m = Machine { home: "/home/h".into(), uid: "1000".into(), gid: "1000".into(), user: "h".into(), os: "Linux".into() };
        let p = serde_json::json!({ "image": "hale-habitat:abc", "listen": "192.168.1.20", "host": "rack-a.lan" });
        let line = in_image(&m, &p, &["up", "--name", "x"], false);
        assert!(line.starts_with("mkdir -p '/home/h/.local/share/hale' && sg=$(docker run --rm --entrypoint stat -v /var/run/docker.sock:/var/run/docker.sock 'hale-habitat:abc' -c %g /var/run/docker.sock) && exec docker run --rm --network host --label 'hale.habitat.dir=/home/h/.local/share/hale/habitat/local' --user '1000':'1000' --group-add \"$sg\""), "{line}");
        assert!(line.contains("-e 'HALE_DNA_HABITAT_LISTEN=192.168.1.20'") && line.contains("-e 'HALE_DNA_HABITAT_ADDRESS=rack-a.lan'"), "{line}");
        assert!(line.contains("-e 'HALE_VAULT_DIR=/home/h/.local/share/hale/vault'"), "{line}");
        assert!(line.contains("-e 'HALE_DNA_HABITAT_EXPECT='"), "{line}");
        assert!(line.ends_with("'hale-habitat:abc' habitat 'up' '--name' 'x'"), "{line}");
    }
}
