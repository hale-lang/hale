//! An organism's own host commands, run on a host the habitat manages:
//! `hale habitat organism place <organism> <command> --host <h>` and
//! `unplace`. The organism declares each in its manifest, and the habitat
//! knows nothing of what it does:
//!
//! ```text
//! [dna.host.node]
//! platforms = ["linux", "macos"]
//! root = true
//! install   = ["hale", "run", "node", "install", "--id", "{id}", "--api", "{api_url}"]
//! uninstall = ["hale", "run", "node", "uninstall", "--id", "{id}"]
//! inputs = ["id", "api_url"]
//! api_service = "api"                                  # whose published URL is {api_url} (default "api")
//! service = "voice-node-{id}"                          # what the install leaves running: a systemd unit or launchd label
//! requires = { linux = ["docker", "gpu"], macos = ["gpu"] }   # what the host must have (or a list: on every platform)
//!
//! [dna.secrets.operator-token]                         # a secret the command reads (dna/secrets.rs)
//! kind = "drawn"
//! readers = ["api", "host.node"]
//! ```
//!
//! The habitat provisions what any host command needs to run at all, which
//! is the same for every organism: the hale toolchain at the organism's pin
//! (its manifest's `[dna] toolchain`, else the habitat's own), installed from the release into a
//! directory of the habitat's on the host, one per version, and what that
//! toolchain builds with (git, curl, tar, clang, OpenSSL's and zlib's
//! headers), with the host's package manager. What only the organism knows
//! (a GPU's drivers, its runtimes) is its command's own.
//!
//! The command runs in the organism's source at the revision the habitat
//! pins for its sapling, cloned on the host, with the toolchain it declares.
//! Everything runs over the person's own SSH logins, as `host add` makes the
//! habitat's account; `--host local`, the habitat's own machine, runs each
//! script here with `sh` when the habitat runs on this machine, else over
//! the login the pointer names (`Target`). Each secret the seed declares
//! with `host.<command>` among its readers is read from the habitat's vault
//! (here, or on the habitat's machine over the person's login through the
//! pointer), held in this process's memory alone, and written over the
//! login's stdin into the organism's vault directory on the target
//! (`~/.local/share/hale/hosted/<organism>.vault`, mode 700, each entry
//! 600) for the run alone: the command is handed `HALE_VAULT_DIR` and each
//! `HALE_SECRET_<NAME>` (a family's: its members' entries, one a line)
//! through `env` (sudo resets the environment), beside the habitat's name
//! (`HALE_DNA_HABITAT_NAME`) and the memory the habitat's own containers
//! on the host may hold (`HALE_DNA_HOST_RESERVED_BYTES`, `reserved.rs`),
//! which every run is handed, and
//! the entries are removed however it ends. A value never reaches a command
//! line, this machine's disk or the head. A command that needs root runs
//! under the person's sudo, which asks at their terminal.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

/// What an organism declares for one host command.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HostCommand {
    pub platforms: Vec<String>,
    pub root: bool,
    pub install: Vec<String>,
    pub uninstall: Vec<String>,
    pub inputs: Vec<String>,
    pub api_service: String,
    /// What the install leaves running on the host, outside compose: the
    /// system service's name (a systemd unit on Linux, a launchd label in
    /// the system domain on macOS), `{id}` filled in; "" when it names none.
    pub service: String,
    /// What the host must have before the command runs (`REQUIRES`), by
    /// platform; a list declares the same on every platform.
    pub requires: BTreeMap<String, Vec<String>>,
}

/// What a host command may require of a host, each checked from what the
/// host says of itself before anything is installed or run.
const REQUIRES: &[&str] = &["docker", "systemd", "nvidia-gpu", "apple-silicon", "gpu"];

/// The inputs a host command may take, by name.
const INPUTS: &[&str] = &["id", "api_url"];

/// What a declaration of the operator token before `[dna.secrets]` is told.
const SUPERSEDED: &str = "superseded by [dna.secrets]: declare the secret and name host.<command> among its readers";
const PLATFORMS: &[&str] = &["linux", "macos"];

/// The host commands a manifest declares (`[dna.host.<name>]`), checked.
pub(super) fn host_commands(manifest: &str) -> Result<BTreeMap<String, HostCommand>, String> {
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| format!("hale.toml: {e}"))?;
    let mut out = BTreeMap::new();
    let Some(hosts) = doc.get("dna").and_then(|d| d.get("host")) else { return Ok(out) };
    let hosts = hosts.as_table().ok_or("[dna.host] is a table of host commands")?;
    for (name, t) in hosts {
        let t = t.as_table().ok_or_else(|| format!("[dna.host.{name}] is a table"))?;
        let strs = |k: &str| -> Result<Vec<String>, String> {
            match t.get(k) {
                None => Ok(Vec::new()),
                Some(v) => v.as_array().ok_or_else(|| format!("[dna.host.{name}] {k} is a list of strings"))?.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("[dna.host.{name}] {k} is a list of strings"))).collect(),
            }
        };
        if t.contains_key("operator_token") {
            return Err(format!("[dna.host.{name}] operator_token is {SUPERSEDED}"));
        }
        for k in t.keys() {
            if !["platforms", "root", "install", "uninstall", "inputs", "api_service", "service", "requires", "readings"].contains(&k.as_str()) {
                return Err(format!("[dna.host.{name}] has no `{k}`"));
            }
        }
        super::placements::check_readings(name, t)?;
        let c = HostCommand {
            platforms: strs("platforms")?,
            root: t.get("root").map(|v| v.as_bool().ok_or_else(|| format!("[dna.host.{name}] root is true or false"))).transpose()?.unwrap_or(false),
            install: strs("install")?,
            uninstall: strs("uninstall")?,
            inputs: strs("inputs")?,
            api_service: t.get("api_service").and_then(|v| v.as_str()).unwrap_or("api").to_string(),
            service: match t.get("service") {
                None => String::new(),
                Some(v) => v.as_str().ok_or_else(|| format!("[dna.host.{name}] service is a string"))?.to_string(),
            },
            requires: match t.get("requires") {
                None => BTreeMap::new(),
                Some(toml::Value::Array(_)) => {
                    let all = strs("requires")?;
                    PLATFORMS.iter().map(|p| (p.to_string(), all.clone())).collect()
                }
                Some(toml::Value::Table(by)) => {
                    let mut out = BTreeMap::new();
                    for (platform, v) in by {
                        if !PLATFORMS.contains(&platform.as_str()) {
                            return Err(format!("[dna.host.{name}] requires.{platform}: a platform is linux or macos"));
                        }
                        let list = v.as_array().ok_or_else(|| format!("[dna.host.{name}] requires.{platform} is a list of strings"))?.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("[dna.host.{name}] requires.{platform} is a list of strings"))).collect::<Result<Vec<_>, _>>()?;
                        out.insert(platform.clone(), list);
                    }
                    out
                }
                Some(_) => return Err(format!("[dna.host.{name}] requires is a list, or a table of lists by platform")),
            },
        };
        if let Some(q) = c.requires.values().flatten().find(|q| !REQUIRES.contains(&q.as_str())) {
            return Err(format!("[dna.host.{name}] requires `{q}`: one of {}", REQUIRES.join(", ")));
        }
        if c.service.contains(|ch: char| !(ch.is_ascii_alphanumeric() || "-_.@{}".contains(ch))) {
            return Err(format!("[dna.host.{name}] service `{}`: letters, digits, `-`, `_`, `.`, `@` and `{{id}}`", c.service));
        }
        if c.install.is_empty() || c.uninstall.is_empty() {
            return Err(format!("[dna.host.{name}] names its install and its uninstall"));
        }
        if let Some(p) = c.platforms.iter().find(|p| !PLATFORMS.contains(&p.as_str())) {
            return Err(format!("[dna.host.{name}] platform `{p}`: linux or macos"));
        }
        if c.inputs.iter().any(|i| i == "operator_token_file") || c.install.iter().chain(&c.uninstall).any(|a| a.contains("{operator_token_file}")) {
            return Err(format!("[dna.host.{name}] input operator_token_file is {SUPERSEDED}"));
        }
        if let Some(i) = c.inputs.iter().find(|i| !INPUTS.contains(&i.as_str())) {
            return Err(format!("[dna.host.{name}] input `{i}`: one of {}", INPUTS.join(", ")));
        }
        out.insert(name.clone(), c);
    }
    Ok(out)
}

/// `argv` with each `{input}` it names filled in; refused when it names
/// one the command does not declare, or one with no value.
pub(super) fn fill(argv: &[String], declared: &[String], values: &BTreeMap<&str, String>) -> Result<Vec<String>, String> {
    argv.iter()
        .map(|a| {
            let mut out = a.clone();
            for name in INPUTS {
                let token = format!("{{{name}}}");
                if out.contains(&token) {
                    if !declared.iter().any(|d| d == name) {
                        return Err(format!("`{a}` names {token}, which the command does not declare among its inputs"));
                    }
                    let v = values.get(name).filter(|v| !v.is_empty()).ok_or_else(|| format!("no value for {token}"))?;
                    out = out.replace(&token, v);
                }
            }
            Ok(out)
        })
        .collect()
}

/// A word for the remote shell, single-quoted.
pub(super) fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The organism's working tree on the target, under the login's home.
fn tree(organism: &str) -> String {
    format!("$HOME/.local/share/hale/hosted/{organism}")
}

/// What the host is, read before anything is touched: one `<what> <value>`
/// a line. A login's non-interactive shell has a short PATH, so Homebrew's
/// and `/usr/local`'s are added.
pub(super) fn facts_script() -> String {
    format!(
        r#"PATH="$PATH:/usr/local/bin:/opt/homebrew/bin"
echo "home $HOME"; echo "os $(uname -s)"; echo "arch $(uname -m)"
for c in git curl tar; do command -v $c >/dev/null 2>&1 && echo "have $c"; done
case "$(uname -s)" in
Darwin)
  xcode-select -p >/dev/null 2>&1 && echo "have clang" && echo "have zlib"
  for b in /opt/homebrew/bin/brew /usr/local/bin/brew; do [ -x "$b" ] && {{ echo "pkg brew $b"; break; }}; done
  for o in /opt/homebrew/opt/openssl@3 /usr/local/opt/openssl@3; do [ -f "$o/include/openssl/ssl.h" ] && {{ echo "have openssl"; break; }}; done
  [ "$(uname -m)" = arm64 ] && echo "fact apple-silicon" ;;
*)
  command -v clang >/dev/null 2>&1 && echo "have clang"
  [ -f /usr/include/openssl/ssl.h ] && echo "have openssl"
  [ -f /usr/include/zlib.h ] && echo "have zlib"
  for m in apt-get dnf; do command -v $m >/dev/null 2>&1 && {{ echo "pkg $m $(command -v $m)"; break; }}; done
  [ -d /run/systemd/system ] && echo "fact systemd"
  command -v nvidia-smi >/dev/null 2>&1 && nvidia-smi -L 2>/dev/null | grep -q '^GPU ' && echo "fact nvidia-gpu" ;;
esac
docker info >/dev/null 2>&1 && echo "fact docker"
for t in "$HOME"/{TOOLCHAINS}/*/bin/hale; do [ -x "$t" ] && echo "toolchain $(basename "$(dirname "$(dirname "$t")")")"; done
true"#
    )
}

/// Where the habitat keeps the toolchains it installs on a host, under the
/// login's home: one directory per release, `<version>/bin/hale`.
const TOOLCHAINS: &str = ".local/share/hale/toolchains";

/// What the host said of itself (`facts_script`).
#[derive(Debug, Default, PartialEq)]
pub(super) struct Facts {
    pub home: String,
    pub os: String,
    pub arch: String,
    /// git, curl, tar, clang, openssl, zlib: what is there of what a toolchain builds with
    pub have: Vec<String>,
    /// the package manager and its path: apt-get, dnf or brew
    pub pkg: Option<(String, String)>,
    /// what `requires` is checked against
    pub facts: Vec<String>,
    /// the toolchains already installed, by version
    pub toolchains: Vec<String>,
}

pub(super) fn facts(out: &str) -> Facts {
    let mut f = Facts::default();
    for l in out.lines() {
        let (k, v) = l.split_once(' ').unwrap_or((l, ""));
        let v = v.trim().to_string();
        match k {
            "home" => f.home = v,
            "os" => f.os = v,
            "arch" => f.arch = v,
            "have" if !f.have.contains(&v) => f.have.push(v),
            "pkg" => f.pkg = v.split_once(' ').map(|(m, p)| (m.to_string(), p.to_string())),
            "fact" => f.facts.push(v),
            "toolchain" => f.toolchains.push(v),
            _ => {}
        }
    }
    f
}

/// What a toolchain builds with, in the order it is needed.
const PREREQUISITES: &[&str] = &["git", "curl", "tar", "clang", "openssl", "zlib"];

/// The package `pkg` installs a prerequisite as; `None` when it does not
/// (clang on macOS is the Command Line Tools', not Homebrew's).
fn package_of(pkg: &str, what: &str) -> Option<&'static str> {
    Some(match (pkg, what) {
        ("apt-get", "git") | ("dnf", "git") | ("brew", "git") => "git",
        ("apt-get", "curl") | ("dnf", "curl") | ("brew", "curl") => "curl",
        ("apt-get", "tar") | ("dnf", "tar") => "tar",
        ("apt-get", "clang") | ("dnf", "clang") => "clang",
        ("apt-get", "openssl") => "libssl-dev",
        ("dnf", "openssl") => "openssl-devel",
        ("brew", "openssl") => "openssl@3",
        ("apt-get", "zlib") => "zlib1g-dev",
        ("dnf", "zlib") => "zlib-devel",
        _ => return None,
    })
}

/// The prerequisites the host lacks, and the packages that install them:
/// refused, naming what is missing, when the host has no package manager
/// the habitat knows or a prerequisite has no package there.
pub(super) fn missing_packages(f: &Facts) -> Result<(Vec<String>, Vec<&'static str>), String> {
    let missing: Vec<String> = PREREQUISITES.iter().filter(|p| !f.have.iter().any(|h| h == *p)).map(|p| p.to_string()).collect();
    if missing.is_empty() {
        return Ok((missing, Vec::new()));
    }
    let Some((pkg, _)) = &f.pkg else {
        return Err(format!("it lacks {} and has no package manager the habitat knows (apt-get, dnf, Homebrew): install them there, then run this again", missing.join(", ")));
    };
    let mut packages = Vec::new();
    for m in &missing {
        match package_of(pkg, m) {
            Some(p) => packages.push(p),
            None if m == "clang" && pkg == "brew" => return Err("it has no Command Line Tools (clang): run `xcode-select --install` there, then run this again".to_string()),
            None => return Err(format!("it lacks {m}, which {pkg} does not install here: install it, then run this again")),
        }
    }
    Ok((missing, packages))
}

/// The packages installed with the host's package manager: under the
/// person's sudo for apt-get and dnf, as the login for Homebrew (which
/// refuses root).
pub(super) fn packages_script(pkg: &str, path: &str, packages: &[&str], sudo: &str) -> String {
    let list = packages.iter().map(|p| sq(p)).collect::<Vec<_>>().join(" ");
    match pkg {
        "apt-get" => format!("set -e; {sudo} env DEBIAN_FRONTEND=noninteractive {p} update -q; {sudo} env DEBIAN_FRONTEND=noninteractive {p} install -y -q {list}", p = sq(path)),
        "dnf" => format!("set -e; {sudo} {p} install -y -q {list}", p = sq(path)),
        _ => format!("set -e; {p} install -q {list}", p = sq(path)),
    }
}

/// Whether the host has what a command requires: the requirements it lacks.
pub(super) fn unmet(requires: &[String], f: &Facts) -> Vec<String> {
    let has = |x: &str| f.facts.iter().any(|y| y == x);
    requires.iter().filter(|q| !(has(q) || (q.as_str() == "gpu" && (has("nvidia-gpu") || has("apple-silicon"))))).cloned().collect()
}

/// The release target of a host's `uname -s` and `uname -m`, as the
/// release names its archives; `None` where none is built.
pub(super) fn release_target(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("Linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("Linux", "aarch64") | ("Linux", "arm64") => Some("aarch64-unknown-linux-gnu"),
        ("Darwin", "arm64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

/// A pin as its release's tag: `0.30.1` and `v0.30.1` are both `v0.30.1`.
pub(super) fn release_tag(pin: &str) -> String {
    let p = pin.trim();
    if p.starts_with('v') { p.to_string() } else { format!("v{p}") }
}

/// The toolchain `tag` installed from its release under `$HOME/<TOOLCHAINS>/<tag>`:
/// downloaded, checked against the release's sha256, unpacked beside, and
/// moved into place whole, so a failed install leaves nothing half there.
pub(super) fn toolchain_script(tag: &str, target: &str) -> String {
    format!(
        r#"set -e; PATH="$PATH:/usr/local/bin:/opt/homebrew/bin"
t={tag}; a="hale-$t-{target}.tar.gz"; u="https://github.com/hale-lang/hale/releases/download/$t/$a"
dest="$HOME/{TOOLCHAINS}/$t"; w=$(mktemp -d); trap 'rm -rf "$w"' EXIT
curl -fsSL -o "$w/$a" "$u"; curl -fsSL -o "$w/$a.sha256" "$u.sha256"
if command -v sha256sum >/dev/null 2>&1; then (cd "$w" && sha256sum -c "$a.sha256" >/dev/null); else (cd "$w" && shasum -a 256 -c "$a.sha256" >/dev/null); fi
mkdir -p "$w/bin" "$(dirname "$dest")"; tar -xzf "$w/$a" -C "$w/bin"; chmod +x "$w/bin/hale"
rm -rf "$dest.new"; mkdir -p "$dest.new"; mv "$w/bin" "$dest.new/bin"; rm -rf "$dest"; mv "$dest.new" "$dest"
"$dest/bin/hale" --version | head -n 1"#,
        tag = sq(tag),
    )
}

/// The organism's source at `revision` of `repo` in its tree on the target
/// (cloned once, fetched after): its manifest.
pub(super) fn source_script(organism: &str, repo: &str, revision: &str) -> String {
    format!(
        r#"set -e; d="{t}"; mkdir -p "$(dirname "$d")"
if [ -d "$d/.git" ]; then git -C "$d" fetch -q origin; else git clone -q {r} "$d"; fi
git -C "$d" -c advice.detachedHead=false checkout -q {v}
echo "manifest"; cat "$d/hale.toml""#,
        t = tree(organism),
        r = sq(repo),
        v = sq(revision)
    )
}

/// The manifest `source_script` printed.
pub(super) fn source(out: &str) -> String {
    out.split_once("manifest\n").map(|(_, b)| b).unwrap_or("").to_string()
}

/// The toolchain an organism declares it runs on (`[dna] toolchain` in its
/// manifest, as a codebase's `toolchain` is), or `None`.
pub(super) fn pinned_toolchain(manifest: &str) -> Option<String> {
    let t: toml::Table = toml::from_str(manifest).ok()?;
    t.get("dna")?.get("toolchain")?.as_str().map(str::to_string).filter(|s| !s.trim().is_empty())
}

/// The platform word `uname -s` answers for.
pub(super) fn platform_of(uname: &str) -> &'static str {
    match uname {
        "Linux" => "linux",
        "Darwin" => "macos",
        _ => "",
    }
}

/// The organism's tree under `home` on the target, spelled out.
fn tree_at(home: &str, organism: &str) -> String {
    format!("{home}/.local/share/hale/hosted/{organism}")
}

/// The organism's vault directory on the target, beside its tree: where the
/// secrets a host command reads are put for its run (mode 700, each entry
/// 600), and removed from when it ends.
fn vault_at(home: &str, organism: &str) -> String {
    format!("{}.vault", tree_at(home, organism))
}

/// One secret a host command reads: its vault entry (a family's: its
/// members' common prefix, `<entry>--`), the variable the command is handed
/// the entries' names in, and the entries handed (a family's members, in
/// their labels' order, `members`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Handed {
    pub name: String,
    pub entry: String,
    pub var: String,
    pub family: bool,
    /// declared `optional`: its readers do without it
    pub optional: bool,
    pub entries: Vec<String>,
}

impl Handed {
    /// The labels of a family's members handed, in order.
    pub(super) fn labels(&self) -> Vec<&str> {
        self.entries.iter().filter_map(|e| e.strip_prefix(self.entry.as_str())).collect()
    }
}

/// The secrets the seed declares with `host.<command>` among their readers
/// (a family's members not yet known: `members`).
pub(super) fn handed(declared: &BTreeMap<String, crate::dna::secrets::Declared>, command: &str, record_id: &str) -> Vec<Handed> {
    declared
        .iter()
        .filter(|(_, d)| d.hosts().contains(&command))
        .map(|(name, d)| {
            let entry = crate::dna::secrets::vault_entry(record_id, name);
            if d.family {
                Handed { name: name.clone(), entry: format!("{entry}--"), var: crate::dna::secrets::env_var(name), family: true, optional: d.optional, entries: Vec::new() }
            } else {
                Handed { name: name.clone(), entry: entry.clone(), var: crate::dna::secrets::env_var(name), family: false, optional: d.optional, entries: vec![entry] }
            }
        })
        .collect()
}

/// A family's members the vault holds (`held`, as `habitat_vault` read it
/// for its prefix `prefix`), each a value under a label, in the labels' order.
pub(super) fn members(held: &BTreeMap<String, Option<String>>, prefix: &str) -> Vec<String> {
    held.iter().filter(|(e, v)| v.is_some() && e.strip_prefix(prefix).is_some_and(crate::dna::secrets::label_ok)).map(|(e, _)| e.clone()).collect()
}

/// What a run is handed of `secrets`, given what the habitat's vault holds
/// (`held`, as `habitat_vault` read their entries): the secrets handed (a
/// family with every member it holds; an optional family with none, and a
/// required one an uninstall goes on without, handed EMPTY: its variable
/// set, so a reader tells "none handed over" from "not declared"), a line
/// per secret it does without (an optional slot unset, never handed; on an
/// uninstall, a required one the vault lacks), and the refusal an install
/// meets for a required secret the vault lacks (a slot no person filled, a
/// family with no member, a drawn secret never drawn). An uninstall is never
/// refused for a missing secret.
pub(super) fn settle(mut secrets: Vec<Handed>, held: &BTreeMap<String, Option<String>>, install: bool, organism: &str) -> (Vec<Handed>, Vec<String>, Option<String>) {
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    let mut unset = Vec::new();
    for h in secrets.iter_mut() {
        if h.family {
            h.entries = members(held, &h.entry);
        }
        let present = if h.family { !h.entries.is_empty() } else { matches!(held.get(&h.entry), Some(Some(_))) };
        if present {
            kept.push(h.clone());
            continue;
        }
        let remedy = if h.family {
            format!("`hale habitat organism {organism} dna secret set {} --member <label>`", h.name)
        } else {
            format!("`hale habitat organism {organism} dna secret set {}` for a slot, `hale habitat organism {organism} dna upgrade` for a drawn one", h.name)
        };
        if h.optional || !install {
            let why = if h.optional { "optional" } else { "the uninstall goes on without it" };
            if h.family {
                skipped.push(format!("{}: a family with no member ({why}), handed empty", h.name));
                kept.push(h.clone());
            } else {
                skipped.push(format!("{}: unset ({why}), not handed", h.name));
            }
        } else if h.family {
            unset.push(format!("{} (a family with no member: {remedy})", h.name));
        } else {
            unset.push(format!("{} (unset: {remedy})", h.name));
        }
    }
    let refusal = (!unset.is_empty()).then(|| format!("it reads secrets the habitat's vault does not hold: {}", unset.join(", ")));
    (kept, skipped, refusal)
}

/// The script that writes one entry into the vault directory `vault` on the
/// target, its value on stdin: the directory made 700 and the entry 600
/// before a byte is written.
pub(super) fn vault_put_script(vault: &str, entry: &str) -> String {
    format!("umask 077; mkdir -p {d} && chmod 700 {d} && cat > {f} && chmod 600 {f}", d = sq(vault), f = sq(&format!("{vault}/{entry}")))
}

/// The variable a host command is handed the habitat's name in, as an
/// organism's services are (`dna::HABITAT_NAME_ENV`).
const HABITAT_NAME_ENV: &str = "HALE_DNA_HABITAT_NAME";

/// The habitat a host command is run for: the one this machine's pointer
/// names when there is one, else this machine's own.
fn habitat_name() -> String {
    super::remote::pointer().and_then(|p| p["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string)).unwrap_or_else(super::habitat_label)
}

/// The command run on the target, in the organism's tree at `dir`: handed
/// the habitat's name (`HALE_DNA_HABITAT_NAME`, `habitat`; none when it is
/// empty), what of the host's memory the habitat holds
/// (`HALE_DNA_HOST_RESERVED_BYTES`, `reserved`; `reserved.rs`),
/// `HALE_VAULT_DIR` (`vault`) and each secret's
/// `HALE_SECRET_<NAME>` through `env`, which sudo keeps where it resets the
/// inherited environment; the entries (written before) removed however it
/// ends, and the directory with them once empty; `sudo` with the
/// provisioned `hale` by its path when the command needs root.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_script(dir: &str, argv: &[String], hale: &str, root: bool, sudo: &str, habitat: &str, reserved: u64, vault: &str, secrets: &[Handed]) -> String {
    let mut words: Vec<String> = argv.iter().map(|a| sq(a)).collect();
    if argv.first().map(String::as_str) == Some("hale") {
        words[0] = sq(hale);
    }
    let run = words.join(" ");
    let mut vars: Vec<String> = Vec::new();
    if !habitat.is_empty() {
        vars.push(sq(&format!("{HABITAT_NAME_ENV}={habitat}")));
    }
    vars.push(sq(&format!("{}={reserved}", super::reserved::RESERVED_ENV)));
    let trap = if secrets.is_empty() {
        String::new()
    } else {
        // an entry's name is the vault's (letters, digits, `-`, `_`): safe as
        // a word; a family's variable holds its members' entries, one a line
        let files = secrets.iter().flat_map(|h| &h.entries).map(|e| format!("\"$vd\"/{e}")).collect::<Vec<_>>().join(" ");
        vars.push(sq(&format!("HALE_VAULT_DIR={vault}")));
        vars.extend(secrets.iter().map(|h| sq(&format!("{}={}", h.var, h.entries.join("\n")))));
        format!("vd={}; trap 'rm -f {files}; rmdir \"$vd\" 2>/dev/null' EXIT; ", sq(vault))
    };
    let env = format!("env {} ", vars.join(" "));
    format!(
        r#"cd {t} || exit 1; {trap}{s}{env}{run}"#,
        t = sq(dir),
        s = if root { format!("{sudo} ") } else { String::new() },
    )
}

/// What the plan says of the habitat's share of `host`'s memory: its own
/// services' limits (`own`) and its organisms' allocations' there.
fn reserve_line(host: &str, own: u64, organisms: u64) -> String {
    use super::reserved::{mib, RESERVED_ENV};
    let total = own + organisms;
    let what = match (own > 0, organisms > 0) {
        (false, false) => format!("the habitat runs nothing on {host} that declares a memory limit"),
        (true, false) => format!("{} on {host}: the habitat's own services' memory limits", mib(total)),
        (false, true) => format!("{} on {host}: the memory limits of its organisms' allocations there", mib(total)),
        (true, true) => format!("{} on {host}: the habitat's own services' memory limits ({}) and its organisms' allocations' there ({})", mib(total), mib(own), mib(organisms)),
    };
    format!("{what}, handed as {RESERVED_ENV}={total}")
}

/// The script that says, of each of `entries` in the vault directory
/// `dir`, whether it holds a value (`missing <entry>`, or `entry <entry>
/// <bytes>`) and, with `values`, the value itself right after its line. An
/// entry ending in `--` is a family's prefix: each entry under it is said.
pub(super) fn vault_script(dir: &str, entries: &[String], values: bool) -> String {
    let list = entries.iter().map(|e| sq(e)).collect::<Vec<_>>().join(" ");
    format!(
        r#"d={d}; say() {{ f="$d/$1"; if [ -s "$f" ]; then echo "entry $1 $(wc -c < "$f" | tr -d ' ')"; {cat}else echo "missing $1"; fi; }}; for n in {list}; do case "$n" in *--) for m in "$d/$n"*; do [ -f "$m" ] && say "${{m##*/}}"; done;; *) say "$n";; esac; done; true"#,
        d = sq(dir),
        cat = if values { r#"cat "$f"; "# } else { "" },
    )
}

/// What `vault_script` printed: each entry's value (`None` when the vault
/// holds none; `Some("")` for one present when values were not asked).
pub(super) fn vault_said(out: &str, values: bool) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut got = BTreeMap::new();
    let mut rest = out;
    while !rest.is_empty() {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        rest = after;
        if let Some(n) = line.strip_prefix("missing ") {
            got.insert(n.to_string(), None);
        } else if let Some(e) = line.strip_prefix("entry ") {
            let (n, len) = e.rsplit_once(' ').ok_or("the vault said something unreadable")?;
            let len: usize = len.parse().map_err(|_| "the vault said something unreadable".to_string())?;
            if values {
                let value = rest.get(..len).ok_or("the vault's answer ended early")?;
                rest = &rest[len..];
                got.insert(n.to_string(), Some(value.to_string()));
            } else {
                got.insert(n.to_string(), Some(String::new()));
            }
        } else if !line.is_empty() {
            return Err("the vault said something unreadable".to_string());
        }
    }
    Ok(got)
}

/// The habitat's vault, asked of `entries`: on its machine over the
/// person's login when this machine points at a habitat elsewhere, else
/// this machine's own (`HALE_VAULT_DIR`, else the cache's). A value is read
/// into memory alone.
fn habitat_vault(entries: &[String], values: bool) -> Result<BTreeMap<String, Option<String>>, String> {
    if entries.is_empty() {
        return Ok(BTreeMap::new());
    }
    if let Some(p) = super::remote::pointer() {
        let out = super::remote::vault_on_machine(&p, |dir| vault_script(dir, entries, values))?;
        return vault_said(&out, values);
    }
    let dir = crate::dna::vault_local_dir().ok_or("the vault is served over HTTP (HALE_VAULT_ADDR): a host command is handed a secret from a local vault alone")?;
    let mut got = BTreeMap::new();
    // a family's prefix asks for each entry under it
    let mut asked: Vec<String> = Vec::new();
    for e in entries {
        if e.ends_with("--") {
            let listed = std::fs::read_dir(&dir).map(|r| r.filter_map(|d| d.ok()).filter_map(|d| d.file_name().to_str().map(str::to_string)).filter(|n| n.starts_with(e.as_str())).collect::<Vec<_>>()).unwrap_or_default();
            asked.extend(listed);
        } else {
            asked.push(e.clone());
        }
    }
    for e in &asked {
        let v = std::fs::read(dir.join(e)).ok().filter(|b| !b.is_empty());
        let v = match v {
            None => None,
            Some(b) => Some(if values { String::from_utf8(b).map_err(|_| format!("the vault's {e} is not text"))? } else { String::new() }),
        };
        got.insert(e.clone(), v);
    }
    Ok(got)
}

/// The name the habitat's rows give its own machine (an allocation's host,
/// a placement's): `--host local` places there.
pub(super) const LOCAL: &str = "local";

/// Whether `name` names the habitat's own machine: `local` when the habitat
/// runs on this one, or the name it is reached by, whole or by its first
/// label (`rack-a` for `rack-a.lan`):
/// the pointer's host when the habitat runs on another machine (`up
/// --host`), else the habitat's address where it runs, or this machine's
/// own hostname for a habitat on this one.
pub(super) fn names_own_machine(name: &str, pointer: Option<&serde_json::Value>) -> bool {
    // `local` is the machine the person types on: the habitat's own only
    // when the habitat runs on it
    if name == LOCAL {
        return pointer.is_none();
    }
    let mut names: Vec<String> = Vec::new();
    match pointer.and_then(|p| p["host"].as_str()).filter(|h| !h.is_empty()) {
        Some(h) => names.push(h.to_string()),
        None => {
            let at = super::habitat_address();
            if at != "127.0.0.1" && at != "localhost" {
                names.push(at);
            }
            if let Some(h) = this_hostname() {
                names.push(h);
            }
        }
    }
    names_one_of(name, &names)
}

/// Whether `name` is one of `names`, whole or by its first label (a bare
/// name never matches a dotted one's later labels).
fn names_one_of(name: &str, names: &[String]) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name) || (!name.contains('.') && n.split('.').next().is_some_and(|l| l.eq_ignore_ascii_case(name))))
}

fn this_hostname() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    let h = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !h.is_empty()).then_some(h)
}

/// The host `name` as the habitat's rows call it. `local` is the machine
/// the person types on: the habitat's own (`local` in its rows) when the
/// habitat runs here; through a pointer, the host the habitat added that is
/// this machine (`this_machine_host`), refused, naming the one `host add`
/// that makes it one, when there is none. Any other name: a host the
/// habitat added, by that name; else `local` when it names the habitat's
/// own machine (`names_own_machine`); else `name` as given (refused where
/// it is used).
pub(super) fn host_key(name: &str, pointer: Option<&serde_json::Value>, hosts: impl FnOnce() -> Result<Vec<serde_json::Value>, String>) -> Result<String, String> {
    if name == LOCAL && pointer.is_none() {
        return Ok(LOCAL.to_string());
    }
    let rows = hosts()?;
    if name == LOCAL {
        return this_machine_host(&rows, is_this_machine).ok_or_else(|| not_a_host_here(pointer));
    }
    if rows.iter().any(|h| h["name"] == name) {
        return Ok(name.to_string());
    }
    Ok(if names_own_machine(name, pointer) { LOCAL.to_string() } else { name.to_string() })
}

/// The host the habitat added that is this machine (`here` says whether an
/// address is one of this machine's), by its name: the first `ready` row
/// whose address resolves to one, `None` for none.
pub(super) fn this_machine_host(rows: &[serde_json::Value], here: impl Fn(&str) -> bool) -> Option<String> {
    rows.iter()
        .filter(|h| h["state"] == "ready")
        .find(|h| here(h["address"].as_str().filter(|a| !a.is_empty()).or(h["name"].as_str()).unwrap_or("")))
        .and_then(|h| h["name"].as_str().map(str::to_string))
}

/// Whether `address` (a name or an IP) resolves to an address of this
/// machine's own, the loopback aside: one a socket here can bind.
fn is_this_machine(address: &str) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(addrs) = (address, 0u16).to_socket_addrs() else { return false };
    addrs.into_iter().any(|a| !a.ip().is_loopback() && std::net::UdpSocket::bind((a.ip(), 0)).is_ok())
}

/// Why `--host local` has nowhere to go: this machine points at a habitat
/// on another and is no host of it yet.
fn not_a_host_here(pointer: Option<&serde_json::Value>) -> String {
    let at = pointer.and_then(|p| p["host"].as_str()).unwrap_or("another machine");
    format!(
        "`local` is this machine, and it is no host of the habitat, which runs on {at}: add it once, by a name {at} reaches it at (`hale habitat host add <this machine's name>`; on macOS, Remote Login on), and `--host local` places here; `--host {at}` is the habitat's own machine"
    )
}

/// Where a host command's scripts run: over the person's own SSH login to
/// a host, or here, with `sh`, when the host is the habitat's own machine
/// and the habitat runs on this one. The scripts are the same either way.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Target {
    Ssh { host: String, user: Option<String>, port: Option<u16> },
    Here,
}

impl Target {
    /// What `--host <host>` names: `local`, the habitat's own machine
    /// (`local_with`); else a host the habitat added (`hosts`, read only
    /// then), `ready`, over the login to it.
    pub(super) fn of(host: &str, pointer: Option<&serde_json::Value>, hosts: impl FnOnce() -> Result<Vec<serde_json::Value>, String>) -> Result<Target, String> {
        if host == LOCAL {
            return Ok(Target::local_with(pointer));
        }
        let row = hosts()?.into_iter().find(|h| h["name"] == host).ok_or_else(|| format!("`{host}` is no host of the habitat (`hale habitat host add {host}`; the habitat's own machine is `--host {LOCAL}`, or its own name)"))?;
        if row["state"] != "ready" {
            return Err(format!("`{host}` is {}, not ready (`hale habitat host probe {host}`)", row["state"].as_str().unwrap_or("unknown")));
        }
        // the host is this machine (a person placing on their own): its
        // scripts run here, with no SSH to oneself
        if pointer.is_some() && this_machine_host(std::slice::from_ref(&row), is_this_machine).is_some() {
            return Ok(Target::Here);
        }
        let port = row["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
        Ok(Target::Ssh { host: host.to_string(), user: None, port })
    }

    /// The habitat's own machine: this one, unless this machine points at
    /// a habitat on another (`pointer`), reached over the pointer's login.
    pub(super) fn local_with(pointer: Option<&serde_json::Value>) -> Target {
        match pointer {
            None => Target::Here,
            Some(p) => Target::Ssh {
                host: p["host"].as_str().unwrap_or("").to_string(),
                user: p["user"].as_str().filter(|u| !u.is_empty()).map(str::to_string),
                port: p["ssh_port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0),
            },
        }
    }

    /// The habitat's own machine, as this machine finds it.
    pub(super) fn local() -> Target {
        Target::local_with(super::remote::pointer().as_ref())
    }

    /// How a line says the scripts get there.
    pub(super) fn how(&self) -> &'static str {
        match self {
            Target::Ssh { .. } => "over your SSH login",
            Target::Here => "on this machine",
        }
    }

    /// A script of the habitat's whose last line is its outcome (`readings`:
    /// `hale-node: …`), made with the `sudo` that suits the run: the outcome,
    /// or why it failed.
    pub(super) fn outcome(&self, script: impl Fn(&str) -> String) -> Result<String, String> {
        match self {
            Target::Ssh { host, user, port } => super::remote::login_script(host, user.as_deref(), *port, script),
            Target::Here => super::readings::run_here(&script(super::readings::sudo_word(std::io::IsTerminal::is_terminal(&std::io::stdin())))),
        }
    }

    /// The command that runs `script` there: `ssh` (`-tt` at a terminal),
    /// or `sh -c` here, without this process's `HALE_*` and `LOTUS_*`
    /// variables (a login there starts without them, and the habitat's vault
    /// directory is no command's).
    fn command(&self, script: &str, terminal: bool) -> Command {
        match self {
            Target::Ssh { host, user, port } => {
                let mut c = Command::new("ssh");
                if let Some(cfg) = std::env::var("HALE_HABITAT_SSH_CONFIG").ok().filter(|v| !v.is_empty()) {
                    c.args(["-F", &cfg]);
                }
                c.args(["-o", "BatchMode=yes"]);
                if terminal {
                    c.arg("-tt");
                }
                if let Some(p) = port {
                    c.args(["-p", &p.to_string()]);
                }
                if let Some(u) = user {
                    c.args(["-l", u]);
                }
                c.arg(host).arg(super::remote::posix(script));
                c
            }
            Target::Here => {
                let mut c = Command::new("sh");
                c.args(["-c", script]);
                for (k, _) in std::env::vars_os() {
                    if k.to_str().is_some_and(|k| k.starts_with("HALE_") || k.starts_with("LOTUS_")) {
                        c.env_remove(k);
                    }
                }
                c
            }
        }
    }

    /// `script` run there, with `stdin` handed to it when given (never at a
    /// terminal), else at a terminal when this is one (sudo asks there). Its
    /// output, or why it failed.
    fn run(&self, script: &str, stdin: Option<&str>, terminal: bool) -> Result<String, String> {
        use std::io::Write;
        let (prog, at) = match self {
            Target::Ssh { host, .. } => ("ssh", format!("on {host}")),
            Target::Here => ("sh", "here".to_string()),
        };
        let mut c = self.command(script, terminal);
        if terminal {
            let st = c.stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().map_err(|e| format!("{prog}: {e}"))?;
            return if st.success() { Ok(String::new()) } else { Err(format!("it failed {at} (exit {})", st.code().unwrap_or(-1))) };
        }
        let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("{prog}: {e}"))?;
        if let (Some(text), Some(mut w)) = (stdin, child.stdin.take()) {
            w.write_all(text.as_bytes()).map_err(|e| format!("{prog}: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("{prog}: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("it failed and said nothing").to_string())
        }
    }
}

/// `hale habitat organism place|unplace <organism> <command> --host <h> [--id <id>] [--plan]`.
pub(super) fn place(args: &[String], install: bool) -> std::process::ExitCode {
    use std::process::ExitCode;
    let verb = if install { "place" } else { "unplace" };
    let usage = format!("usage: hale habitat organism {verb} <organism> <command> --host <host>|local [--id <id>] [--plan]");
    let (Some(organism), Some(name)) = (args.first().filter(|a| !a.starts_with('-')), args.get(1).filter(|a| !a.starts_with('-'))) else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    let (mut host, mut id, mut plan) = (None, None, false);
    let mut i = 2;
    while i < args.len() {
        match (args[i].as_str(), args.get(i + 1)) {
            ("--host", Some(h)) => {
                host = Some(h.clone());
                i += 2;
            }
            ("--id", Some(v)) => {
                id = Some(v.clone());
                i += 2;
            }
            ("--plan", _) => {
                plan = true;
                i += 1;
            }
            _ => {
                eprintln!("{usage}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(host) = host else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    match place_lines(organism, name, &host, id.as_deref(), plan, install) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("hale habitat organism {verb} {organism} {name}: {why}");
            ExitCode::from(1)
        }
    }
}

fn rows(member: &str, field: &str) -> Result<Vec<serde_json::Value>, String> {
    let v = super::sapling::head_call(member, serde_json::json!({ "limit": 500 }))?;
    Ok(v[field].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default())
}

pub(super) fn place_lines(organism: &str, name: &str, host: &str, id: Option<&str>, plan: bool, install: bool) -> Result<(), String> {
    // the organism, its sapling's source, and the target host, as the head knows them
    let index = super::sapling::head_call("Commands::index", serde_json::json!({ "limit": 500 }))?;
    let organisms: Vec<serde_json::Value> = index["organisms"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    let key = organisms.iter().find(|o| o["name"] == organism).and_then(|o| o["key"].as_str()).map(|k| k[..12.min(k.len())].to_string()).ok_or_else(|| format!("the habitat grafted no organism `{organism}`"))?;
    let sapling = rows("Commands::saplings", "saplings")?.into_iter().find(|s| s["name"] == organism).ok_or_else(|| format!("`{organism}` was not grafted from a sapling the habitat pins: a host command runs the organism's source at its sapling's pinned revision"))?;
    let (repo, revision) = (sapling["repo"].as_str().unwrap_or("").to_string(), sapling["revision"].as_str().unwrap_or("").to_string());
    // where its scripts run: the habitat's own machine (`local`) here or
    // over the pointer's login, any other host over the login to it
    // the habitat's own machine by any name it answers to is `local`
    let host = &host_key(host, super::remote::pointer().as_ref(), || rows("Commands::hosts", "hosts"))?;
    let host = host.as_str();
    let at = Target::of(host, super::remote::pointer().as_ref(), || rows("Commands::hosts", "hosts"))?;
    // the host as a person knows it: the habitat's own machine by its name
    let shown = super::shown_host(host);
    let id = id.map(str::to_string).unwrap_or_else(|| shown.split('.').next().unwrap_or(&shown).to_string());

    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());

    // what the host is, read before anything is touched
    super::progress(&format!("reading what {shown} is, {}", at.how()));
    let f = facts(&at.run(&facts_script(), None, false)?);
    if f.home.is_empty() {
        return Err(format!("{shown} named no home for your login"));
    }
    let release = release_target(&f.os, &f.arch).ok_or_else(|| format!("{shown} is {} {}, for which hale builds no release", f.os, f.arch))?;

    // what any toolchain builds with: installed first, since the source needs git
    let (missing, packages) = missing_packages(&f).map_err(|w| format!("{shown}: {w}"))?;
    let builds = if packages.is_empty() {
        format!("{:<8}git, curl, tar, clang, OpenSSL and zlib are on {shown}", "builds")
    } else {
        let (pkg, path) = f.pkg.clone().unwrap_or_default();
        let via = if pkg == "brew" { "Homebrew".to_string() } else { format!("{pkg}, through your sudo") };
        if plan {
            let line = format!("{:<8}{shown} lacks {}: would install {} with {via}", "builds", missing.join(", "), packages.join(" "));
            if missing.iter().any(|m| m == "git") {
                println!("{line}");
                println!("{:<8}the rest of the plan reads {organism}'s source on {shown}, which needs git there first", "note");
                return Ok(());
            }
            line
        } else {
            if pkg != "brew" && !tty {
                return Err(format!("{shown} lacks {}, and installing them needs your sudo, which asks at a terminal: run it at one", missing.join(", ")));
            }
            super::progress(&format!("installing {} on {shown} with {via}", packages.join(" ")));
            at.run(&packages_script(&pkg, &path, &packages, "sudo"), None, tty && pkg != "brew")?;
            format!("{:<8}{} installed on {shown} with {via}", "builds", packages.join(" "))
        }
    };

    // its source on the target: the declaration and the toolchain it pins
    super::progress(&format!("fetching {organism}'s source at {revision} on {shown}"));
    let manifest = source(&at.run(&source_script(organism, &repo, &revision), None, false)?);
    let commands = host_commands(&manifest)?;
    let cmd = commands.get(name).ok_or_else(|| format!("{organism} declares no host command `{name}` at {revision} ([dna.host.{name}]{})", if commands.is_empty() { String::new() } else { format!("; it declares {}", commands.keys().cloned().collect::<Vec<_>>().join(", ")) }))?;
    let platform = platform_of(&f.os);
    if !cmd.platforms.is_empty() && !cmd.platforms.iter().any(|x| x == platform) {
        return Err(format!("`{name}` runs on {}; {shown} is {}", cmd.platforms.join(" and "), if f.os.is_empty() { "unknown" } else { &f.os }));
    }
    let requires = cmd.requires.get(platform).cloned().unwrap_or_default();
    let unmet = unmet(&requires, &f);
    if !unmet.is_empty() {
        return Err(format!("`{name}` requires {} on {platform}; {shown} lacks {}", requires.join(", "), unmet.join(", ")));
    }
    // the toolchain it declares, else the habitat's own (this one's)
    let pinned = pinned_toolchain(&manifest);
    let tag = release_tag(pinned.as_deref().unwrap_or(env!("CARGO_PKG_VERSION")));
    let whose = if pinned.is_some() { format!("{organism}'s pin") } else { format!("{organism} declares none (`[dna] toolchain`): the habitat's own") };
    let have_toolchain = f.toolchains.iter().any(|t| *t == tag);
    let dir = tree_at(&f.home, organism);
    let hale = format!("{}/{TOOLCHAINS}/{tag}/bin/hale", f.home);

    // its inputs: the id, the api's URL where its service is published
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    values.insert("id", id.clone());
    let needs = |i: &str| cmd.inputs.iter().any(|x| x == i);
    // the secrets it reads ([dna.secrets.<name>] naming host.<command>)
    let declared = crate::dna::secrets::declared(&manifest)?;
    let reads_secrets = declared.values().any(|d| d.hosts().contains(&name));
    let allocations = if needs("api_url") || reads_secrets { rows("Commands::allocations", "allocations")? } else { Vec::new() };
    if needs("api_url") {
        let services = allocations.iter().find(|a| a["instance"].as_str().is_some_and(|k| k.starts_with(&key)) && a["role"] == "services").ok_or_else(|| format!("{organism} has no services allocation: its application runs nowhere yet"))?;
        let links = services["links"].as_array().cloned().unwrap_or_default();
        let url = links.iter().find(|l| l["service"] == cmd.api_service.as_str()).and_then(|l| l["url"].as_str()).filter(|u| !u.is_empty()).map(str::to_string);
        values.insert("api_url", url.ok_or_else(|| format!("{organism}'s `{}` publishes no address another host reaches", cmd.api_service))?);
    }
    let secrets = if reads_secrets {
        // the organism's record, which its vault entries are named for
        let record = allocations.iter().filter(|a| a["instance"].as_str().is_some_and(|k| k.starts_with(&key))).find_map(|a| a["organism"].as_str().filter(|o| o.len() >= 12)).ok_or_else(|| format!("the habitat holds no allocation of {organism} naming its record, which its secrets' vault entries are named for"))?;
        handed(&declared, name, record)
    } else {
        Vec::new()
    };
    let vault = vault_at(&f.home, organism);
    // which of them the habitat's vault holds, asked before anything runs:
    // a required one it lacks refuses an install (`--plan` too), never an
    // uninstall; an optional one is done without
    let held = habitat_vault(&secrets.iter().map(|h| h.entry.clone()).collect::<Vec<_>>(), false)?;
    let (secrets, skipped, refusal) = settle(secrets, &held, install, organism);
    if let Some(why) = refusal {
        return Err(format!("`{name}` {why}"));
    }
    let argv = fill(if install { &cmd.install } else { &cmd.uninstall }, &cmd.inputs, &values)?;
    let service = if cmd.service.is_empty() { String::new() } else { fill(&[cmd.service.clone()], &cmd.inputs, &values)?.remove(0) };

    // what of the host's memory the habitat itself holds, read from the
    // compose files on its machine (here, or over the pointer's login)
    super::progress(&format!("reading what the habitat runs on {shown}"));
    let pointer = super::remote::pointer();
    let habitats_machine = host == LOCAL || pointer.as_ref().is_some_and(|p| p["host"] == host);
    let listed = match &pointer {
        Some(p) => super::remote::habitat_on_machine(p, super::reserved::files_script),
        None => Target::Here.run(&super::reserved::files_script(&super::local_habitat_dir()), None, false),
    }
    .map_err(|w| format!("reading what the habitat runs on {shown}: {w}"))?;
    let (own, organisms) = super::reserved::reserved(&super::reserved::files_said(&listed)?, host, habitats_machine);

    println!("{:<8}{organism}'s `{name}` on {shown} ({platform}{}), as {organism} {revision} declares it", if install { "place" } else { "unplace" }, if cmd.root { ", as root through your sudo" } else { "" });
    if host == LOCAL {
        println!("{:<8}{} the habitat's own machine, reached {}", "host", if shown == LOCAL { "this machine is".to_string() } else { format!("{shown} is") }, match &at {
            Target::Ssh { host: h, .. } => format!("over your SSH login to {h}"),
            Target::Here => at.how().to_string(),
        });
    } else if at == Target::Here {
        println!("{:<8}{shown} is this machine, a host of the habitat: its scripts run here", "host");
    }
    println!("{:<8}{}", "runs", argv.join(" "));
    println!("{builds}");
    if !requires.is_empty() {
        println!("{:<8}{} on {platform}: {shown} has them", "needs", requires.join(", "));
    }
    println!("{:<8}{tag}, {whose}: {}", "hale", if have_toolchain { format!("in {}/{TOOLCHAINS}/{tag} on {shown}", f.home) } else if plan { format!("would install it in {}/{TOOLCHAINS}/{tag} from its release, checked against its sha256", f.home) } else { format!("installing it in {}/{TOOLCHAINS}/{tag} from its release", f.home) });
    println!("{:<8}in {dir} on {shown}, with {hale}", "from");
    if !secrets.is_empty() {
        let said = secrets.iter().map(|h| if h.family { format!("{} (a family: {})", h.name, h.labels().join(", ")) } else { h.name.clone() }).collect::<Vec<_>>().join(", ");
        println!("{:<8}{said}, from the habitat's vault into {vault} on {shown} (700, each 600) for the run alone, handed as HALE_VAULT_DIR and {}", "secrets", secrets.iter().map(|h| h.var.as_str()).collect::<Vec<_>>().join(", "));
    }
    for s in &skipped {
        println!("{:<8}{s}", "skips");
    }
    println!("{:<8}{}", "reserve", reserve_line(&shown, own, organisms));
    if !service.is_empty() {
        println!("{:<8}{service}, {} on {shown}", "service", if platform == "macos" { "a launchd service in the system domain" } else { "a systemd unit" });
    }
    println!("{}", super::placements::plan_line(host, &manifest, name, install));
    if plan {
        return Ok(());
    }
    if cmd.root && !tty {
        return Err("it needs root, and sudo asks at a terminal: run it at one".to_string());
    }
    if !have_toolchain {
        super::progress(&format!("installing hale {tag} on {shown}"));
        let said = at.run(&toolchain_script(&tag, release), None, false)?;
        println!("{:<8}{tag} installed on {shown} ({})", "hale", said.lines().last().unwrap_or("").trim());
    }
    if secrets.iter().any(|h| !h.entries.is_empty()) {
        // each value from the habitat's vault, held here in memory alone,
        // onto the target over the login's stdin
        super::progress(&format!("handing {organism}'s secrets to {shown}"));
        let entries: Vec<String> = secrets.iter().flat_map(|h| h.entries.iter().cloned()).collect();
        let values = habitat_vault(&entries, true)?;
        for e in &entries {
            let value = values.get(e).cloned().flatten().ok_or_else(|| format!("the habitat's vault no longer holds {e}"))?;
            if let Err(why) = at.run(&vault_put_script(&vault, e), Some(&value), false) {
                // what was written goes again
                let _ = at.run(&format!("rm -rf {}", sq(&vault)), None, false);
                return Err(format!("{e} into {vault} on {shown}: {why}"));
            }
        }
    }
    super::progress(&format!("running {organism}'s `{name}` on {shown}"));
    let script = run_script(&dir, &argv, &hale, cmd.root, "sudo", &habitat_name(), own + organisms, &vault, &secrets);
    at.run(&script, None, tty)?;
    println!("{:<8}{organism}'s `{name}` {} on {shown}", "done", if install { "placed" } else { "taken off" });
    if !service.is_empty() {
        // what the command says it leaves running, as the host's own service manager sees it
        let state = service_state(&at.run(&service_script(&service), None, false)?);
        let want = if install { "running" } else { "absent" };
        if state != want {
            return Err(format!("{organism}'s `{name}` finished, but its service {service} on {shown} is {state}, not {want}"));
        }
        println!("{:<8}{service} on {shown}: {state}", "service");
    }
    super::placements::record(organism, name, host, &id, &service, platform, &manifest, &tag, install)?;
    Ok(())
}

/// What the host's service manager says of `service`: `running`, `stopped`
/// (known, not running) or `absent`, a line of its own.
fn service_script(service: &str) -> String {
    let s = sq(service);
    format!(
        "if [ \"$(uname -s)\" = Darwin ]; then out=$(launchctl print system/{s} 2>/dev/null) || {{ echo 'state absent'; exit 0; }}; case \"$out\" in *'state = running'*) echo 'state running';; *) echo 'state stopped';; esac; \
         else st=$(systemctl is-active {s} 2>/dev/null); case \"$st\" in active|activating|reloading) echo 'state running';; *) if systemctl cat {s} >/dev/null 2>&1; then echo 'state stopped'; else echo 'state absent'; fi;; esac; fi"
    )
}

fn service_state(out: &str) -> String {
    out.lines().find_map(|l| l.strip_prefix("state ")).map(|s| s.trim().to_string()).unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_habitats_own_machine_answers_to_its_name() {
        let pointer = serde_json::json!({ "host": "habitat-box.lab.lan", "name": "h" });
        for n in ["habitat-box.lab.lan", "habitat-box", "HABITAT-BOX"] {
            assert!(super::names_own_machine(n, Some(&pointer)), "{n}");
        }
        // `local` is the machine the person types on: the habitat's own
        // only when the habitat runs there
        assert!(super::names_own_machine("local", None));
        for n in ["local", "gpu-1.lab.lan", "lab", "habitat-box.other.lan", "box"] {
            assert!(!super::names_own_machine(n, Some(&pointer)), "{n}");
        }
        // a host the habitat added keeps its own name, whatever it looks like
        let added = || Ok(vec![serde_json::json!({ "name": "habitat-box", "state": "ready" })]);
        assert_eq!(super::host_key("habitat-box", Some(&pointer), added).unwrap(), "habitat-box");
        let none = || Ok(Vec::new());
        assert_eq!(super::host_key("habitat-box.lab.lan", Some(&pointer), none).unwrap(), "local");
        assert_eq!(super::host_key("gpu-1.lab.lan", Some(&pointer), || Ok(Vec::new())).unwrap(), "gpu-1.lab.lan");
        // with no pointer `local` is the habitat's own machine, as ever
        assert_eq!(super::host_key("local", None, || Ok(Vec::new())).unwrap(), "local");
        // through a pointer, `local` is this machine: the added host that is
        // it, or refused naming the `host add` that makes it one
        let rows = vec![
            serde_json::json!({ "name": "gpu-1.lab.lan", "address": "gpu-1.lab.lan", "state": "ready" }),
            serde_json::json!({ "name": "desk.lab.lan", "address": "desk.lab.lan", "state": "ready" }),
        ];
        assert_eq!(super::this_machine_host(&rows, |a| a == "desk.lab.lan").as_deref(), Some("desk.lab.lan"));
        assert_eq!(super::this_machine_host(&rows, |_| false), None);
        let why = super::host_key("local", Some(&pointer), || Ok(Vec::new())).unwrap_err();
        assert!(why.contains("no host of the habitat, which runs on habitat-box.lab.lan") && why.contains("hale habitat host add") && why.contains("`--host habitat-box.lab.lan`"), "{why}");
    }

    use super::*;

    const VOICE: &str = r#"
[dna]
needs = ["memory"]

[dna.host.node]
platforms = ["linux", "macos"]
root = true
install   = ["hale", "run", "node", "install", "--id", "{id}", "--api", "{api_url}"]
uninstall = ["hale", "run", "node", "uninstall", "--id", "{id}"]
inputs = ["id", "api_url"]
service = "voice-node-{id}"

[dna.secrets.operator-token]
kind = "drawn"
readers = ["api", "host.node"]

[dna.secrets.upstream-key]
kind = "slot"
readers = ["api"]
"#;

    #[test]
    fn a_host_command_is_read_and_checked() {
        let c = host_commands(VOICE).unwrap();
        let node = &c["node"];
        assert!(node.root && node.platforms == ["linux", "macos"] && node.api_service == "api" && node.service == "voice-node-{id}");
        assert!(host_commands(&VOICE.replace("voice-node-{id}", "voice node")).unwrap_err().contains("service"));
        assert!(host_commands("[dna]\nneeds = []\n").unwrap().is_empty());
        let refuse = |extra: &str| host_commands(&VOICE.replace("root = true", extra)).unwrap_err();
        assert!(refuse("root = true\ncolor = \"red\"").contains("has no `color`"));
        assert!(refuse("root = true\nplatforms = [\"windows\"]").contains("platform") || host_commands(&VOICE.replace("[\"linux\", \"macos\"]", "[\"windows\"]")).is_err());
    }

    #[test]
    fn the_operator_token_of_old_is_superseded_by_declared_secrets() {
        let old = VOICE.replace("service = ", "operator_token = [\"voice-api\", \"admin-token\"]\nservice = ");
        let why = host_commands(&old).unwrap_err();
        assert!(why.contains("operator_token is superseded by [dna.secrets]") && why.contains("host.<command> among its readers"), "{why}");
        let why = host_commands(&VOICE.replace("inputs = [\"id\", \"api_url\"]", "inputs = [\"id\", \"api_url\", \"operator_token_file\"]")).unwrap_err();
        assert!(why.contains("operator_token_file is superseded by [dna.secrets]"), "{why}");
        let why = host_commands(&VOICE.replace("\"{id}\", \"--api\"", "\"{id}\", \"--token\", \"{operator_token_file}\", \"--api\"")).unwrap_err();
        assert!(why.contains("superseded"), "{why}");
        // api_service and {api_url} stay
        let c = host_commands(&VOICE.replace("service = ", "api_service = \"web\"\nservice = ")).unwrap();
        assert_eq!(c["node"].api_service, "web");
    }

    #[test]
    fn a_host_command_is_handed_the_secrets_it_reads_and_no_other() {
        let declared = crate::dna::secrets::declared(VOICE).unwrap();
        let id = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let h = handed(&declared, "node", id);
        let entry = format!("secret-dna_{}-operator-token", &id[..56]);
        assert_eq!(h, vec![Handed { name: "operator-token".into(), entry: entry.clone(), var: crate::dna::secrets::env_var("operator-token"), family: false, optional: false, entries: vec![entry] }]);
        assert!(handed(&declared, "other", id).is_empty());
        // a family: every member the vault holds, in its labels' order
        let fam = crate::dna::secrets::declared(&VOICE.replace("[dna.secrets.upstream-key]\nkind = \"slot\"\nreaders = [\"api\"]", "[dna.secrets.model-token]\nkind = \"slot\"\nfamily = true\nreaders = [\"host.node\"]")).unwrap();
        let mut h = handed(&fam, "node", id);
        let prefix = format!("secret-dna_{}-model-token--", &id[..56]);
        assert!(h[0].family && h[0].entry == prefix && h[0].entries.is_empty(), "{h:?}");
        let mut held = BTreeMap::new();
        held.insert(format!("{prefix}work"), Some(String::new()));
        held.insert(format!("{prefix}home"), Some(String::new()));
        held.insert(format!("{prefix}gone"), None);
        held.insert(format!("{prefix}x.tmp"), Some(String::new()));
        h[0].entries = members(&held, &prefix);
        assert_eq!(h[0].labels(), ["home", "work"]);
        let s = run_script("/d", &["x".into()], "/h", false, "sudo", "", 0, "/v", &h);
        assert!(s.contains(&format!("'{}={prefix}home\n{prefix}work'", crate::dna::secrets::env_var("model-token"))), "{s}");
        assert!(s.contains(&format!("\"$vd\"/{prefix}home \"$vd\"/{prefix}work")), "{s}");
        assert!(Command::new("sh").args(["-n", "-c", &s]).status().unwrap().success(), "{s}");
    }

    /// What a run is handed when the vault lacks a secret: an install is
    /// refused for a required one alone; an optional slot is not handed, an
    /// optional family is handed empty (set); an uninstall is never refused.
    #[test]
    fn an_optional_secret_is_done_without_and_an_uninstall_is_never_refused() {
        let toml = format!(
            "{VOICE}\n[dna.secrets.extra]\nkind = \"slot\"\noptional = true\nreaders = [\"host.node\"]\n\n[dna.secrets.tokens]\nkind = \"slot\"\nfamily = true\noptional = true\nreaders = [\"host.node\"]\n\n[dna.secrets.keys]\nkind = \"slot\"\nfamily = true\nreaders = [\"host.node\"]\n"
        );
        let declared = crate::dna::secrets::declared(&toml).unwrap();
        let id = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let all = handed(&declared, "node", id);
        let entry = |n: &str| crate::dna::secrets::vault_entry(id, n);
        let names = |v: &[Handed]| v.iter().map(|h| h.name.clone()).collect::<Vec<_>>();
        // the vault holds the drawn token and one member of the required family
        let mut held = BTreeMap::new();
        held.insert(entry("operator-token"), Some(String::new()));
        held.insert(entry("extra"), None);
        held.insert(format!("{}--a", entry("keys")), Some(String::new()));
        let (kept, skipped, refusal) = settle(all.clone(), &held, true, "voice");
        assert_eq!(refusal, None);
        assert_eq!(names(&kept), ["keys", "operator-token", "tokens"]);
        let tokens = kept.iter().find(|h| h.name == "tokens").unwrap();
        assert!(tokens.entries.is_empty(), "an optional family with no member is handed empty");
        assert_eq!(skipped, ["extra: unset (optional), not handed", "tokens: a family with no member (optional), handed empty"]);
        // its variable is set, and empty; the optional slot's is not set at all
        let s = run_script("/d", &["x".into()], "/h", true, "sudo", "", 0, "/v", &kept);
        assert!(s.contains(&format!("'{}='", crate::dna::secrets::env_var("tokens"))), "{s}");
        assert!(!s.contains(&crate::dna::secrets::env_var("extra")), "{s}");
        assert!(Command::new("sh").args(["-n", "-c", &s]).status().unwrap().success(), "{s}");
        // a required secret the vault lacks refuses an install, naming its remedy
        let (_, _, refusal) = settle(all.clone(), &BTreeMap::new(), true, "voice");
        let why = refusal.unwrap();
        assert!(why.contains("operator-token (unset: ") && why.contains("keys (a family with no member: `hale habitat organism voice dna secret set keys --member <label>`)"), "{why}");
        assert!(!why.contains("extra") && !why.contains("tokens"), "an optional one never refuses: {why}");
        // and never an uninstall: what exists is handed, the rest said
        let (kept, skipped, refusal) = settle(all, &BTreeMap::new(), false, "voice");
        assert_eq!(refusal, None);
        assert_eq!(names(&kept), ["keys", "tokens"]);
        assert!(skipped.contains(&"operator-token: unset (the uninstall goes on without it), not handed".to_string()), "{skipped:?}");
        assert!(skipped.contains(&"keys: a family with no member (the uninstall goes on without it), handed empty".to_string()), "{skipped:?}");
    }

    #[test]
    fn inputs_fill_their_places_and_nothing_undeclared_does() {
        let c = &host_commands(VOICE).unwrap()["node"];
        let mut v = BTreeMap::new();
        v.insert("id", "gpu-1".to_string());
        v.insert("api_url", "http://10.0.0.5:8080".to_string());
        let argv = fill(&c.install, &c.inputs, &v).unwrap();
        assert_eq!(argv.join(" "), "hale run node install --id gpu-1 --api http://10.0.0.5:8080");
        assert!(fill(&["{api_url}".to_string()], &["id".to_string()], &v).unwrap_err().contains("does not declare"));
        v.remove("api_url");
        assert!(fill(&c.install, &c.inputs, &v).unwrap_err().contains("no value"));
    }

    #[test]
    fn a_host_says_what_it_is_and_what_it_lacks() {
        let f = facts("home /home/x\nos Linux\narch aarch64\nhave git\nhave curl\nhave tar\nhave clang\nhave zlib\npkg apt-get /usr/bin/apt-get\nfact systemd\nfact nvidia-gpu\nfact docker\ntoolchain v0.30.0\n");
        assert_eq!((f.home.as_str(), f.os.as_str(), f.arch.as_str()), ("/home/x", "Linux", "aarch64"));
        assert_eq!(f.pkg, Some(("apt-get".to_string(), "/usr/bin/apt-get".to_string())));
        assert_eq!(f.toolchains, ["v0.30.0"]);
        // a host lacking only OpenSSL's headers
        assert_eq!(missing_packages(&f).unwrap(), (vec!["openssl".to_string()], vec!["libssl-dev"]));
        let dnf = Facts { pkg: Some(("dnf".into(), "/usr/bin/dnf".into())), ..facts("have curl\nhave tar\n") };
        assert_eq!(missing_packages(&dnf).unwrap().1, ["git", "clang", "openssl-devel", "zlib-devel"]);
        assert!(missing_packages(&facts("have curl\n")).unwrap_err().contains("no package manager"));
        let mac = facts("os Darwin\narch arm64\nhave git\nhave curl\nhave tar\npkg brew /opt/homebrew/bin/brew\nfact apple-silicon\n");
        assert!(missing_packages(&mac).unwrap_err().contains("xcode-select --install"));
        let mac = Facts { have: vec!["git".into(), "curl".into(), "tar".into(), "clang".into(), "zlib".into()], ..mac };
        assert_eq!(missing_packages(&mac).unwrap().1, ["openssl@3"]);
        assert!(missing_packages(&Facts { have: PREREQUISITES.iter().map(|p| p.to_string()).collect(), ..Facts::default() }).unwrap().1.is_empty());
    }

    #[test]
    fn requires_is_checked_against_the_host() {
        let gpu = facts("fact nvidia-gpu\nfact docker\nfact systemd\n");
        let mac = facts("fact apple-silicon\n");
        let req = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(unmet(&req(&["gpu", "docker", "systemd"]), &gpu).is_empty());
        assert!(unmet(&req(&["gpu"]), &mac).is_empty());
        assert_eq!(unmet(&req(&["gpu", "docker"]), &mac), ["docker"]);
        assert_eq!(unmet(&req(&["nvidia-gpu"]), &mac), ["nvidia-gpu"]);
        let flat = host_commands(&VOICE.replace("root = true", "root = true\nrequires = [\"gpu\", \"docker\"]")).unwrap();
        assert_eq!((flat["node"].requires["linux"].clone(), flat["node"].requires["macos"].clone()), (req(&["gpu", "docker"]), req(&["gpu", "docker"])));
        // voice's: docker on Linux alone, where its runtimes are containers
        let by = host_commands(&VOICE.replace("root = true", "root = true\nrequires = { linux = [\"docker\", \"nvidia-gpu\"], macos = [\"apple-silicon\"] }")).unwrap();
        assert_eq!(by["node"].requires["linux"], req(&["docker", "nvidia-gpu"]));
        assert_eq!(by["node"].requires["macos"], req(&["apple-silicon"]));
        assert!(unmet(&by["node"].requires["macos"], &mac).is_empty());
        assert!(host_commands(&VOICE.replace("root = true", "root = true\nrequires = { windows = [\"gpu\"] }")).unwrap_err().contains("requires.windows"));
        assert!(host_commands(&VOICE.replace("root = true", "root = true\nrequires = { linux = [\"tpu\"] }")).unwrap_err().contains("requires `tpu`"));
        assert!(host_commands(&VOICE.replace("root = true", "root = true\nrequires = [\"tpu\"]")).unwrap_err().contains("requires `tpu`"));
    }

    #[test]
    fn the_toolchain_is_the_organisms_pin_from_its_release() {
        assert_eq!(pinned_toolchain("[dna]\nneeds = []\ntoolchain = \"v0.30.1\"\n").as_deref(), Some("v0.30.1"));
        assert_eq!(pinned_toolchain("[dna]\nneeds = []\n"), None);
        assert_eq!((release_tag("0.30.1"), release_tag("v0.30.1")), ("v0.30.1".to_string(), "v0.30.1".to_string()));
        assert_eq!(release_target("Linux", "aarch64"), Some("aarch64-unknown-linux-gnu"));
        assert_eq!(release_target("Darwin", "arm64"), Some("aarch64-apple-darwin"));
        assert_eq!(release_target("Darwin", "x86_64"), None);
        assert_eq!(source("Cloning…\nmanifest\n[dna]\nneeds = []\n"), "[dna]\nneeds = []\n");
    }

    #[test]
    fn the_plan_names_the_habitats_share_of_the_host() {
        assert_eq!(reserve_line("gpu", 0, 0), "the habitat runs nothing on gpu that declares a memory limit, handed as HALE_DNA_HOST_RESERVED_BYTES=0");
        assert_eq!(reserve_line("gpu", 0, 512 << 20), "512 MiB on gpu: the memory limits of its organisms' allocations there, handed as HALE_DNA_HOST_RESERVED_BYTES=536870912");
        assert_eq!(reserve_line("local", 1920 << 20, 128 << 20), "2048 MiB on local: the habitat's own services' memory limits (1920 MiB) and its organisms' allocations' there (128 MiB), handed as HALE_DNA_HOST_RESERVED_BYTES=2147483648");
    }

    #[test]
    fn the_run_uses_the_hosts_hale_under_sudo_and_hands_the_secrets_through_env() {
        let vault = vault_at("/home/x", "voice");
        assert_eq!(vault, "/home/x/.local/share/hale/hosted/voice.vault");
        let secrets = vec![Handed { name: "operator-token".into(), entry: "secret-dna_ab-operator-token".into(), var: crate::dna::secrets::env_var("operator-token"), family: false, optional: false, entries: vec!["secret-dna_ab-operator-token".into()] }];
        let argv: Vec<String> = vec!["hale".into(), "run".into(), "node".into(), "install".into()];
        let s = run_script("/home/x/.local/share/hale/hosted/voice", &argv, "/usr/local/bin/hale", true, "sudo", "home", 2415919104, &vault, &secrets);
        assert!(s.starts_with("cd '/home/x/.local/share/hale/hosted/voice' || exit 1; vd='/home/x/.local/share/hale/hosted/voice.vault'; trap 'rm -f \"$vd\"/secret-dna_ab-operator-token; rmdir \"$vd\" 2>/dev/null' EXIT; "), "{s}");
        // sudo resets the environment: `env` after it hands them over, the
        // habitat's name first, then the habitat's share of the host's memory
        let token = crate::dna::secrets::env_var("operator-token");
        assert!(s.contains(&format!("sudo env 'HALE_DNA_HABITAT_NAME=home' 'HALE_DNA_HOST_RESERVED_BYTES=2415919104' 'HALE_VAULT_DIR=/home/x/.local/share/hale/hosted/voice.vault' '{token}=secret-dna_ab-operator-token' '/usr/local/bin/hale' 'run' 'node' 'install'")), "{s}");
        // as the login, the same through env; with no secret, the habitat's
        // name and share alone and no trap; with no name, the share alone
        // (0 when the habitat runs nothing there: always handed)
        let user = run_script("/d", &argv, "/h", false, "sudo", "home", 0, &vault, &secrets);
        assert!(user.contains("; env 'HALE_DNA_HABITAT_NAME=home' 'HALE_DNA_HOST_RESERVED_BYTES=0' 'HALE_VAULT_DIR=") && !user.contains("sudo"), "{user}");
        let named = run_script("/d", &argv, "/h", true, "sudo", "home", 0, &vault, &[]);
        assert_eq!(named, "cd '/d' || exit 1; sudo env 'HALE_DNA_HABITAT_NAME=home' 'HALE_DNA_HOST_RESERVED_BYTES=0' '/h' 'run' 'node' 'install'");
        let quoted = run_script("/d", &argv, "/h", false, "sudo", "it's", 0, &vault, &[]);
        assert!(Command::new("sh").args(["-n", "-c", &quoted]).status().unwrap().success(), "{quoted}");
        assert!(quoted.contains("env 'HALE_DNA_HABITAT_NAME=it'\\''s' "), "{quoted}");
        let bare = run_script("/d", &argv, "/h", true, "sudo", "", 0, &vault, &[]);
        assert_eq!(bare, "cd '/d' || exit 1; sudo env 'HALE_DNA_HOST_RESERVED_BYTES=0' '/h' 'run' 'node' 'install'");
        // the value goes on stdin, never into the script
        let put = vault_put_script(&vault, "secret-dna_ab-operator-token");
        assert_eq!(put, "umask 077; mkdir -p '/home/x/.local/share/hale/hosted/voice.vault' && chmod 700 '/home/x/.local/share/hale/hosted/voice.vault' && cat > '/home/x/.local/share/hale/hosted/voice.vault/secret-dna_ab-operator-token' && chmod 600 '/home/x/.local/share/hale/hosted/voice.vault/secret-dna_ab-operator-token'");
        let asked = vault_script("/v", &["a".to_string(), "b".to_string()], true);
        assert_eq!(service_state("noise\nstate running\n"), "running");
        assert_eq!(service_state(""), "unknown");
        for script in [
            s,
            user,
            put,
            asked,
            vault_script("/v", &["a".to_string()], false),
            source_script("voice", "https://github.com/hale-lang/voice", "v1.2.0"),
            service_script("voice-node-gpu-1"),
            facts_script(),
            toolchain_script("v0.30.1", "aarch64-unknown-linux-gnu"),
            packages_script("apt-get", "/usr/bin/apt-get", &["libssl-dev", "clang"], "sudo"),
            packages_script("brew", "/opt/homebrew/bin/brew", &["openssl@3"], "sudo"),
        ] {
            assert!(Command::new("sh").args(["-n", "-c", &script]).status().unwrap().success(), "{script}");
        }
    }

    /// `--host local` is the habitat's own machine: here when the habitat
    /// runs on this machine, over the pointer's login when it runs on
    /// another; any other name a host the habitat added, over the login to it.
    #[test]
    fn the_host_names_where_the_scripts_run() {
        let unread = || -> Result<Vec<serde_json::Value>, String> { panic!("the habitat's own machine is no host row") };
        assert_eq!(Target::of(LOCAL, None, unread).unwrap(), Target::Here);
        let pointer = serde_json::json!({ "name": "home", "host": "habitat-box.lan", "user": "op", "ssh_port": 2222, "head": "http://habitat-box.lan:8801" });
        assert_eq!(Target::of(LOCAL, Some(&pointer), unread).unwrap(), Target::Ssh { host: "habitat-box.lan".into(), user: Some("op".into()), port: Some(2222) });
        let bare = serde_json::json!({ "host": "habitat-box.lan", "user": "", "ssh_port": 0 });
        assert_eq!(Target::local_with(Some(&bare)), Target::Ssh { host: "habitat-box.lan".into(), user: None, port: None });
        // a host the habitat added, pointer or not
        let hosts = || Ok(vec![serde_json::json!({ "name": "gpu-1.lan", "state": "ready", "port": 2200 }), serde_json::json!({ "name": "new.lan", "state": "probing", "port": 0 })]);
        for p in [None, Some(&pointer)] {
            assert_eq!(Target::of("gpu-1.lan", p, hosts).unwrap(), Target::Ssh { host: "gpu-1.lan".into(), user: None, port: Some(2200) });
        }
        let why = Target::of("other.lan", None, hosts).unwrap_err();
        assert_eq!(why, "`other.lan` is no host of the habitat (`hale habitat host add other.lan`; the habitat's own machine is `--host local`, or its own name)");
        assert!(Target::of("new.lan", None, hosts).unwrap_err().contains("is probing, not ready"));
        assert_eq!((Target::Here.how(), Target::of(LOCAL, Some(&pointer), unread).unwrap().how()), ("on this machine", "over your SSH login"));
        // the transport: ssh with the login's user and port (the script
        // run by sh, whatever the login shell), or sh here
        let ssh = Target::Ssh { host: "habitat-box.lan".into(), user: Some("op".into()), port: Some(2222) }.command("true", false);
        let args: Vec<_> = ssh.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(ssh.get_program(), "ssh");
        assert!(args.ends_with(&["-p".into(), "2222".into(), "-l".into(), "op".into(), "habitat-box.lan".into(), "exec /bin/sh -c 'true'".into()]), "{args:?}");
        let here = Target::Here.command("true", false);
        assert_eq!((here.get_program().to_string_lossy().into_owned(), here.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>()), ("sh".to_string(), vec!["-c".to_string(), "true".to_string()]));
    }

    /// A script run here is the same script, its output read back, its
    /// stdin handed over, its failure its last words.
    #[test]
    fn a_script_runs_here_as_it_runs_over_a_login() {
        assert_eq!(Target::Here.run("echo placed; echo here", None, false).unwrap(), "placed\nhere\n");
        assert_eq!(Target::Here.run("cat", Some("two\nlines"), false).unwrap(), "two\nlines");
        assert_eq!(Target::Here.run("echo noise; echo first >&2; echo why >&2; exit 3", None, false).unwrap_err(), "why");
        // a secret onto the habitat's own machine, as onto a host: 700 and 600
        let home = std::env::temp_dir().join(format!("hale-hosted-here-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let vault = vault_at(&home.display().to_string(), "voice");
        Target::Here.run(&vault_put_script(&vault, "secret-dna_ab-operator-token"), Some("s3cret"), false).unwrap();
        let entry = std::path::Path::new(&vault).join("secret-dna_ab-operator-token");
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), "s3cret");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&vault).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&entry).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_habitats_vault_says_what_it_holds_and_each_value_whole() {
        let dir = std::env::temp_dir().join(format!("hale-hosted-vault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), "two\nlines").unwrap();
        std::fs::write(dir.join("e"), "").unwrap();
        std::fs::write(dir.join("f--one"), "1").unwrap();
        std::fs::write(dir.join("f--two"), "22").unwrap();
        let entries = vec!["a".to_string(), "b".to_string(), "e".to_string(), "f--".to_string(), "g--".to_string()];
        let run = |values: bool| {
            let out = Command::new("sh").args(["-c", &vault_script(&dir.display().to_string(), &entries, values)]).output().unwrap();
            vault_said(&String::from_utf8(out.stdout).unwrap(), values).unwrap()
        };
        let got = run(true);
        assert_eq!(got["a"].as_deref(), Some("two\nlines"));
        // a family's prefix says each member; one with none says nothing
        assert_eq!((got["f--one"].as_deref(), got["f--two"].as_deref()), (Some("1"), Some("22")));
        assert_eq!(members(&got, "f--"), ["f--one", "f--two"]);
        assert!(members(&got, "g--").is_empty());
        assert_eq!((got["b"].as_deref(), got["e"].as_deref()), (None, None));
        let held = run(false);
        assert_eq!((held["a"].as_deref(), held["b"].as_deref()), (Some(""), None));
        assert!(vault_said("entry a 10\nshort", true).unwrap_err().contains("ended early"));
        assert!(vault_said("noise\n", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
