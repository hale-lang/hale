//! An organism's own host commands, run on a host the habitat manages:
//! `hale habitat organism place <organism> <command> --host <h>` and
//! `unplace`. The organism declares each in its manifest, and the habitat
//! knows nothing of what it does:
//!
//! ```text
//! [dna.host.node]
//! platforms = ["linux", "macos"]
//! root = true
//! install   = ["hale", "run", "node", "install", "--id", "{id}", "--api", "{api_url}", "--operator-token-file", "{operator_token_file}"]
//! uninstall = ["hale", "run", "node", "uninstall", "--id", "{id}", "--operator-token-file", "{operator_token_file}"]
//! inputs = ["id", "api_url", "operator_token_file"]
//! api_service = "api"                                  # whose published URL is {api_url} (default "api")
//! operator_token = ["voice-api", "admin-token"]        # run in that service's container: prints the token
//! ```
//!
//! The command runs in the organism's source at the revision the habitat
//! pins for its sapling, cloned on the host, with the host's own `hale`.
//! Everything runs over the person's own SSH logins, as `host add` makes the
//! habitat's account: the operator token is read in the api's container on
//! the host it runs on, written to a 0600 file on the target, handed to the
//! command, and removed; it never reaches this machine's disk or the head.
//! A command that needs root runs under the person's sudo, which asks at
//! their terminal.

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
    pub operator_token: Vec<String>,
}

/// The inputs a host command may take, by name.
const INPUTS: &[&str] = &["id", "api_url", "operator_token_file"];
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
        for k in t.keys() {
            if !["platforms", "root", "install", "uninstall", "inputs", "api_service", "operator_token"].contains(&k.as_str()) {
                return Err(format!("[dna.host.{name}] has no `{k}`"));
            }
        }
        let c = HostCommand {
            platforms: strs("platforms")?,
            root: t.get("root").map(|v| v.as_bool().ok_or_else(|| format!("[dna.host.{name}] root is true or false"))).transpose()?.unwrap_or(false),
            install: strs("install")?,
            uninstall: strs("uninstall")?,
            inputs: strs("inputs")?,
            api_service: t.get("api_service").and_then(|v| v.as_str()).unwrap_or("api").to_string(),
            operator_token: strs("operator_token")?,
        };
        if c.install.is_empty() || c.uninstall.is_empty() {
            return Err(format!("[dna.host.{name}] names its install and its uninstall"));
        }
        if let Some(p) = c.platforms.iter().find(|p| !PLATFORMS.contains(&p.as_str())) {
            return Err(format!("[dna.host.{name}] platform `{p}`: linux or macos"));
        }
        if let Some(i) = c.inputs.iter().find(|i| !INPUTS.contains(&i.as_str())) {
            return Err(format!("[dna.host.{name}] input `{i}`: one of {}", INPUTS.join(", ")));
        }
        if c.inputs.iter().any(|i| i == "operator_token_file") && c.operator_token.is_empty() {
            return Err(format!("[dna.host.{name}] takes operator_token_file and names no `operator_token` command to read it with"));
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
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The organism's working tree on the target, under the login's home.
fn tree(organism: &str) -> String {
    format!("$HOME/.local/share/hale/hosted/{organism}")
}

/// The organism's source at `revision` of `repo` in its tree on the
/// target (cloned once, fetched after), and what the host is: its
/// manifest, `uname -s`, and the `hale` on its PATH with its version.
pub(super) fn prepare_script(organism: &str, repo: &str, revision: &str) -> String {
    format!(
        r#"set -e; d="{t}"; mkdir -p "$(dirname "$d")"
if [ -d "$d/.git" ]; then git -C "$d" fetch -q origin; else git clone -q {r} "$d"; fi
git -C "$d" -c advice.detachedHead=false checkout -q {v}
echo "home $HOME"; echo "os $(uname -s)"
if h=$(command -v hale); then echo "hale $h $("$h" --version | head -n 1)"; else echo "hale none"; fi
echo "manifest"; cat "$d/hale.toml""#,
        t = tree(organism),
        r = sq(repo),
        v = sq(revision)
    )
}

/// What `prepare_script` said of the host.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Prepared {
    pub home: String,
    pub os: String,
    pub hale: Option<String>,
    pub hale_version: String,
    pub manifest: String,
}

pub(super) fn prepared(out: &str) -> Prepared {
    let mut p = Prepared::default();
    let mut lines = out.lines();
    for l in lines.by_ref() {
        if let Some(h) = l.strip_prefix("home ") {
            p.home = h.trim().to_string();
        } else if let Some(os) = l.strip_prefix("os ") {
            p.os = os.trim().to_string();
        } else if let Some(h) = l.strip_prefix("hale ") {
            if h.trim() != "none" {
                let (path, ver) = h.split_once(' ').unwrap_or((h, ""));
                p.hale = Some(path.to_string());
                p.hale_version = ver.trim().to_string();
            }
        } else if l == "manifest" {
            break;
        }
    }
    p.manifest = lines.collect::<Vec<_>>().join("\n");
    p
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

/// The command run on the target, in the organism's tree at `dir`: the
/// token's file (written before, 0600) removed however it ends; `sudo`
/// with the host's `hale` by its path when the command needs root.
pub(super) fn run_script(dir: &str, argv: &[String], hale: &str, root: bool, sudo: &str, token_file: Option<&str>) -> String {
    let mut words: Vec<String> = argv.iter().map(|a| sq(a)).collect();
    if argv.first().map(String::as_str) == Some("hale") {
        words[0] = sq(hale);
    }
    let run = words.join(" ");
    format!(
        r#"cd {t} || exit 1; {trap}{s}{run}"#,
        t = sq(dir),
        trap = token_file.map(|f| format!("tf={}; trap 'rm -f \"$tf\"' EXIT; ", sq(f))).unwrap_or_default(),
        s = if root { format!("{sudo} ") } else { String::new() },
    )
}

/// Over the person's login to `host`: `script`, with `stdin` handed to it
/// when given (never at a terminal), else at a terminal when this is one
/// (sudo asks there). Its output, or why it failed.
fn over_login(host: &str, port: Option<u16>, script: &str, stdin: Option<&str>, terminal: bool) -> Result<String, String> {
    use std::io::Write;
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
    c.arg(host).arg(script);
    if terminal {
        let st = c.stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().map_err(|e| format!("ssh: {e}"))?;
        return if st.success() { Ok(String::new()) } else { Err(format!("it failed on {host} (exit {})", st.code().unwrap_or(-1))) };
    }
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("ssh: {e}"))?;
    if let (Some(text), Some(mut w)) = (stdin, child.stdin.take()) {
        w.write_all(text.as_bytes()).map_err(|e| format!("ssh: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("it failed and said nothing").to_string())
    }
}

/// The operator token, read in the container of `service` of the organism
/// keyed `key` on `host` (docker by the login's own access, else its sudo).
fn read_token(host: &str, port: Option<u16>, key: &str, service: &str, argv: &[String]) -> Result<String, String> {
    let run = argv.iter().map(|a| sq(a)).collect::<Vec<_>>().join(" ");
    let script = format!(
        r#"if docker info >/dev/null 2>&1; then d=docker; else d="sudo -n docker"; fi
c=$($d ps --filter label=com.docker.compose.service={s} --format '{{{{.Names}}}}' | grep -- "-{k}-" | head -n 1)
[ -n "$c" ] || {{ echo "no running {s} container of the organism {k} on this host" >&2; exit 1; }}
$d exec "$c" {run}"#,
        s = sq(service),
        k = key,
    );
    let out = over_login(host, port, &script, None, false)?;
    let token = out.trim().to_string();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(format!("`{}` in {service} printed no single token", argv.join(" ")));
    }
    Ok(token)
}

/// `hale habitat organism place|unplace <organism> <command> --host <h> [--id <id>] [--plan]`.
pub(super) fn place(args: &[String], install: bool) -> std::process::ExitCode {
    use std::process::ExitCode;
    let verb = if install { "place" } else { "unplace" };
    let usage = format!("usage: hale habitat organism {verb} <organism> <command> --host <host> [--id <id>] [--plan]");
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

fn place_lines(organism: &str, name: &str, host: &str, id: Option<&str>, plan: bool, install: bool) -> Result<(), String> {
    // the organism, its sapling's source, and the target host, as the head knows them
    let index = super::sapling::head_call("Commands::index", serde_json::json!({ "limit": 500 }))?;
    let organisms: Vec<serde_json::Value> = index["organisms"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    let key = organisms.iter().find(|o| o["name"] == organism).and_then(|o| o["key"].as_str()).map(|k| k[..12.min(k.len())].to_string()).ok_or_else(|| format!("the habitat grafted no organism `{organism}`"))?;
    let sapling = rows("Commands::saplings", "saplings")?.into_iter().find(|s| s["name"] == organism).ok_or_else(|| format!("`{organism}` was not grafted from a sapling the habitat pins: a host command runs the organism's source at its sapling's pinned revision"))?;
    let (repo, revision) = (sapling["repo"].as_str().unwrap_or("").to_string(), sapling["revision"].as_str().unwrap_or("").to_string());
    let target = rows("Commands::hosts", "hosts")?.into_iter().find(|h| h["name"] == host).ok_or_else(|| format!("`{host}` is no host of the habitat (`hale habitat host add {host}`)"))?;
    if target["state"] != "ready" {
        return Err(format!("`{host}` is {}, not ready (`hale habitat host probe {host}`)", target["state"].as_str().unwrap_or("unknown")));
    }
    let port = target["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
    let id = id.map(str::to_string).unwrap_or_else(|| host.split('.').next().unwrap_or(host).to_string());

    // its source on the target, and what the target is
    super::progress(&format!("fetching {organism}'s source at {revision} on {host}, over your SSH login"));
    let p = prepared(&over_login(host, port, &prepare_script(organism, &repo, &revision), None, false)?);
    let commands = host_commands(&p.manifest)?;
    let cmd = commands.get(name).ok_or_else(|| format!("{organism} declares no host command `{name}` at {revision} ([dna.host.{name}]{})", if commands.is_empty() { String::new() } else { format!("; it declares {}", commands.keys().cloned().collect::<Vec<_>>().join(", ")) }))?;
    let platform = platform_of(&p.os);
    if !cmd.platforms.is_empty() && !cmd.platforms.iter().any(|x| x == platform) {
        return Err(format!("`{name}` runs on {}; {host} is {}", cmd.platforms.join(" and "), if p.os.is_empty() { "unknown" } else { &p.os }));
    }
    if p.home.is_empty() {
        return Err(format!("{host} named no home for your login"));
    }
    let dir = tree_at(&p.home, organism);
    let hale = p.hale.clone().ok_or_else(|| format!("{host} has no `hale` on its PATH: install the release {organism} pins there, then run this again"))?;

    // its inputs: the id, the api's URL where its service is published, the operator token
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    values.insert("id", id.clone());
    let needs = |i: &str| cmd.inputs.iter().any(|x| x == i);
    let mut api_host = String::new();
    if needs("api_url") || needs("operator_token_file") {
        let services = rows("Commands::allocations", "allocations")?.into_iter().find(|a| a["instance"].as_str().is_some_and(|k| k.starts_with(&key)) && a["role"] == "services").ok_or_else(|| format!("{organism} has no services allocation: its application runs nowhere yet"))?;
        api_host = services["applied"].as_str().filter(|h| !h.is_empty()).or_else(|| services["host"].as_str()).unwrap_or("").to_string();
        let links = services["links"].as_array().cloned().unwrap_or_default();
        let url = links.iter().find(|l| l["service"] == cmd.api_service.as_str()).and_then(|l| l["url"].as_str()).filter(|u| !u.is_empty()).map(str::to_string);
        if needs("api_url") {
            values.insert("api_url", url.ok_or_else(|| format!("{organism}'s `{}` publishes no address another host reaches", cmd.api_service))?);
        }
    }
    let token_file = format!("{dir}/.operator-token-{id}");
    if needs("operator_token_file") {
        values.insert("operator_token_file", token_file.clone());
    }
    let argv = fill(if install { &cmd.install } else { &cmd.uninstall }, &cmd.inputs, &values)?;

    println!("{:<8}{organism}'s `{name}` on {host} ({platform}{}), as {organism} {revision} declares it", if install { "place" } else { "unplace" }, if cmd.root { ", as root through your sudo" } else { "" });
    println!("{:<8}{}", "runs", argv.join(" "));
    println!("{:<8}in {dir} on {host}, with {hale} ({})", "from", p.hale_version);
    if needs("operator_token_file") {
        println!("{:<8}{organism}'s operator token, read in its `{}` on {api_host}, in a 0600 file on {host} for the run alone", "token", cmd.api_service);
    }
    if plan {
        return Ok(());
    }
    if needs("operator_token_file") {
        let (api_login, api_port) = if api_host == "local" || api_host.is_empty() {
            return Err(format!("{organism}'s `{}` runs on the habitat's own machine: reading its token there is not built yet", cmd.api_service));
        } else {
            let hp = rows("Commands::hosts", "hosts")?.into_iter().find(|h| h["name"] == api_host.as_str()).and_then(|h| h["port"].as_u64()).and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
            (api_host.clone(), hp)
        };
        super::progress(&format!("reading {organism}'s operator token on {api_login}"));
        let token = read_token(&api_login, api_port, &key, &cmd.api_service, &cmd.operator_token)?;
        over_login(host, port, &format!("umask 077; cat > {}", sq(&token_file)), Some(&token), false)?;
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if cmd.root && !tty {
        return Err("it needs root, and sudo asks at a terminal: run it at one".to_string());
    }
    super::progress(&format!("running {organism}'s `{name}` on {host}"));
    let script = run_script(&dir, &argv, &hale, cmd.root, "sudo", needs("operator_token_file").then_some(token_file.as_str()));
    over_login(host, port, &script, None, tty)?;
    println!("{:<8}{organism}'s `{name}` {} on {host}", "done", if install { "placed" } else { "taken off" });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOICE: &str = r#"
[dna]
needs = ["memory"]

[dna.host.node]
platforms = ["linux", "macos"]
root = true
install   = ["hale", "run", "node", "install", "--id", "{id}", "--api", "{api_url}", "--operator-token-file", "{operator_token_file}"]
uninstall = ["hale", "run", "node", "uninstall", "--id", "{id}", "--operator-token-file", "{operator_token_file}"]
inputs = ["id", "api_url", "operator_token_file"]
operator_token = ["voice-api", "admin-token"]
"#;

    #[test]
    fn a_host_command_is_read_and_checked() {
        let c = host_commands(VOICE).unwrap();
        let node = &c["node"];
        assert!(node.root && node.platforms == ["linux", "macos"] && node.api_service == "api");
        assert!(host_commands("[dna]\nneeds = []\n").unwrap().is_empty());
        let refuse = |extra: &str| host_commands(&VOICE.replace("root = true", extra)).unwrap_err();
        assert!(refuse("root = true\ncolor = \"red\"").contains("has no `color`"));
        assert!(refuse("root = true\nplatforms = [\"windows\"]").contains("platform") || host_commands(&VOICE.replace("[\"linux\", \"macos\"]", "[\"windows\"]")).is_err());
        assert!(host_commands(&VOICE.replace("operator_token = [\"voice-api\", \"admin-token\"]", "")).unwrap_err().contains("operator_token"));
    }

    #[test]
    fn inputs_fill_their_places_and_nothing_undeclared_does() {
        let c = &host_commands(VOICE).unwrap()["node"];
        let mut v = BTreeMap::new();
        v.insert("id", "gpu-1".to_string());
        v.insert("api_url", "http://10.0.0.5:8080".to_string());
        v.insert("operator_token_file", "/home/x/t".to_string());
        let argv = fill(&c.install, &c.inputs, &v).unwrap();
        assert_eq!(argv.join(" "), "hale run node install --id gpu-1 --api http://10.0.0.5:8080 --operator-token-file /home/x/t");
        assert!(fill(&["{api_url}".to_string()], &["id".to_string()], &v).unwrap_err().contains("does not declare"));
        v.remove("api_url");
        assert!(fill(&c.install, &c.inputs, &v).unwrap_err().contains("no value"));
    }

    #[test]
    fn the_target_says_what_it_is() {
        let p = prepared("home /Users/x\nos Darwin\nhale /opt/hale/bin/hale hale 0.30.0\nmanifest\n[dna]\nneeds = []");
        assert_eq!(p, Prepared { home: "/Users/x".into(), os: "Darwin".into(), hale: Some("/opt/hale/bin/hale".into()), hale_version: "hale 0.30.0".into(), manifest: "[dna]\nneeds = []".into() });
        assert_eq!(prepared("os Linux\nhale none\nmanifest\n").hale, None);
        assert_eq!(platform_of("Darwin"), "macos");
    }

    #[test]
    fn the_run_uses_the_hosts_hale_under_sudo_and_removes_the_token() {
        let s = run_script("/home/x/.local/share/hale/hosted/voice", &["hale".into(), "run".into(), "node".into(), "install".into()], "/usr/local/bin/hale", true, "sudo", Some("/home/x/t"));
        assert!(s.starts_with("cd '/home/x/.local/share/hale/hosted/voice' || exit 1; tf='/home/x/t'; trap 'rm -f \"$tf\"' EXIT; "), "{s}");
        assert!(s.contains("sudo '/usr/local/bin/hale' 'run' 'node' 'install'"), "{s}");
        for script in [s, prepare_script("voice", "https://github.com/hale-lang/voice", "v1.2.0")] {
            assert!(Command::new("sh").args(["-n", "-c", &script]).status().unwrap().success(), "{script}");
        }
    }
}
