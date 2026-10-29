//! The services the habitat placed on its hosts (`hale habitat organism
//! place`, hosted.rs): recorded, observed and taken off again.
//!
//! A placement is recorded through the habitat's head once its host
//! command ran and the service it declares runs (`Commands::record_placed`,
//! a `habitat.placed` row; `unplace` records `habitat.unplaced`,
//! dna/api/habitat_placed.hl), so the habitat knows what it left running
//! where. A command that declares `readings = <port>` serves its own
//! Prometheus readings there (a Hale program's `std::observe::Scrape`):
//! the head registers `http://<host>:<port>/metrics` with the habitat's
//! senses as it records the row, and the store's `services` job scrapes it
//! (the Services dashboard). Nothing polls a host.
//!
//! What placing installed on a host is taken off again by whatever removes
//! the host or the habitat (`host remove`, `organism remove`, `down
//! --forget`): each placement's own `uninstall`, run as `unplace` runs it,
//! over the person's own SSH login (where sudo asks), then the organism's
//! checkout and the toolchain no placement left there uses. One that cannot
//! be taken off is said, with the `unplace` that takes it off, and the rest
//! goes on.

use std::collections::BTreeSet;

use serde_json::{json, Value};

/// A placement as the habitat's record holds it (a `habitat.placed` or
/// `habitat.unplaced` row's body).
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Placement {
    pub organism: String,
    pub command: String,
    pub host: String,
    pub id: String,
    pub service: String,
    pub platform: String,
    pub toolchain: String,
    /// `<address>:<port>` the habitat's senses scrape, "" when it serves
    /// no readings
    pub target: String,
    /// whether it holds (its latest row is `habitat.placed`)
    pub placed: bool,
}

impl Placement {
    pub(super) fn of(body: &Value) -> Placement {
        let s = |k: &str| body[k].as_str().unwrap_or("").to_string();
        Placement {
            organism: s("organism"),
            command: s("command"),
            host: s("host"),
            id: s("id"),
            service: s("service"),
            platform: s("platform"),
            toolchain: s("toolchain"),
            target: s("target"),
            placed: body["state"] != "unplaced",
        }
    }

    /// The record's key of it.
    pub(super) fn key(&self) -> String {
        format!("{}/{}/{}/{}", self.organism, self.command, self.host, self.id)
    }

    /// Where its readings are served, as the habitat's senses scrape them.
    pub(super) fn readings(&self) -> String {
        if self.target.is_empty() { "no readings".to_string() } else { endpoint(&self.target) }
    }

    /// The command that takes it off by hand.
    pub(super) fn unplace_command(&self) -> String {
        format!("hale habitat organism unplace {} {} --host {} --id {}", self.organism, self.command, self.host, self.id)
    }

    /// What it is, in a line: whose command, which service, where.
    pub(super) fn line(&self) -> String {
        let service = if self.service.is_empty() { "no service named".to_string() } else { format!("{} ({})", self.service, kind(&self.platform)) };
        format!("{}'s `{}` {}: {service} on {}, {}", self.organism, self.command, self.id, self.host, self.readings())
    }
}

/// The service manager a placed service runs under, by its platform.
pub(super) fn kind(platform: &str) -> &'static str {
    if platform == "macos" { "launchd" } else { "systemd" }
}

/// The readings endpoint of a scrape target `<address>:<port>`.
pub(super) fn endpoint(target: &str) -> String {
    format!("http://{target}/metrics")
}

// ---------------------------------------------------------------
// the declaration: `[dna.host.<name>] readings = <port>`
// ---------------------------------------------------------------

/// `[dna.host.<name>] readings`, when it declares one: the port its service
/// serves its Prometheus readings on, an integer 1 to 65535.
pub(super) fn check_readings(name: &str, t: &toml::Table) -> Result<(), String> {
    match t.get("readings") {
        None => Ok(()),
        Some(toml::Value::Integer(n)) if (1..=65535).contains(n) => Ok(()),
        Some(v) => Err(format!("[dna.host.{name}] readings = {v}: the port its service serves its readings on (`std::observe::Scrape`), 1 to 65535")),
    }
}

/// The readings port `[dna.host.<name>]` of `manifest` declares, if any.
pub(super) fn readings_port(manifest: &str, name: &str) -> Option<u16> {
    let doc: toml::Table = toml::from_str(manifest).ok()?;
    doc.get("dna")?.get("host")?.get(name)?.get("readings")?.as_integer().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0)
}

/// The address the habitat reaches `host` at, as its row names it.
fn address_of(host: &str) -> String {
    super::sapling::head_call("Commands::hosts", json!({ "limit": 500 }))
        .ok()
        .and_then(|v| v["hosts"].as_str().and_then(|t| serde_json::from_str::<Vec<Value>>(t).ok()))
        .and_then(|hs| hs.into_iter().find(|h| h["name"] == host))
        .and_then(|h| h["address"].as_str().filter(|a| !a.is_empty()).map(str::to_string))
        .unwrap_or_else(|| host.to_string())
}

/// The plan's line for the readings a placement registers (`place`) or
/// unregisters (`unplace`) with the habitat's senses.
pub(super) fn plan_line(host: &str, manifest: &str, name: &str, install: bool) -> String {
    match readings_port(manifest, name) {
        Some(port) => format!(
            "{:<8}{}, {} the habitat's senses (their `services` job)",
            "readings",
            endpoint(&format!("{}:{port}", address_of(host))),
            if install { "registered with" } else { "unregistered from" }
        ),
        None => format!("{:<8}none: `{name}` declares no readings port ([dna.host.{name}] readings)", "readings"),
    }
}

// ---------------------------------------------------------------
// the record
// ---------------------------------------------------------------

/// The placement recorded through the habitat's head once its command ran
/// there: `habitat.placed` (and its readings registered) after `place`,
/// `habitat.unplaced` after `unplace`.
#[allow(clippy::too_many_arguments)]
pub(super) fn record(organism: &str, command: &str, host: &str, id: &str, service: &str, platform: &str, manifest: &str, toolchain: &str, install: bool) -> Result<(), String> {
    let member = if install { "Commands::record_placed" } else { "Commands::record_unplaced" };
    let readings = if install { readings_port(manifest, command).unwrap_or(0) } else { 0 };
    let v = super::sapling::head_call(
        member,
        json!({ "organism": organism, "command": command, "host": host, "id": id, "service": service, "platform": platform, "readings": readings, "toolchain": toolchain }),
    )
    .map_err(|why| format!("it ran on {host}, and the habitat did not record it: {why}"))?;
    if !install && v["code"] == "not_found" {
        // placed before the habitat recorded placements: nothing to unrecord
        println!("{:<8}the habitat's record held no placement of it", "recorded");
        return Ok(());
    }
    if v["ok"] != json!(true) {
        return Err(format!("it ran on {host}, and the habitat did not record it: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or("")));
    }
    if install {
        match v["readings"].as_str().filter(|r| !r.is_empty()) {
            Some(r) => println!("{:<8}{r}, registered with the habitat's senses", "readings"),
            None => println!("{:<8}none registered", "readings"),
        }
    }
    if let Some(why) = v["why"].as_str().filter(|w| !w.is_empty()) {
        println!("{:<8}{why}", "note");
    }
    println!("{:<8}{} in the habitat's record", "recorded", if install { "habitat.placed" } else { "habitat.unplaced" });
    Ok(())
}

/// Each placement's latest row of the record's `rows` (`{kind, entity,
/// body}`, the body a JSON string): those that hold, and every one with
/// its state, in the record's order.
pub(super) fn fold(rows: &[Value]) -> (Vec<Placement>, Vec<Placement>) {
    let mut latest: Vec<(String, Placement)> = Vec::new();
    for r in rows {
        let kind = r["kind"].as_str().unwrap_or("");
        if kind != "habitat.placed" && kind != "habitat.unplaced" {
            continue;
        }
        let body: Value = r["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_else(|| r["body"].clone());
        let mut p = Placement::of(&body);
        p.placed = kind == "habitat.placed";
        let entity = r["entity"].as_str().map(str::to_string).unwrap_or_else(|| p.key());
        latest.retain(|(e, _)| *e != entity);
        latest.push((entity, p));
    }
    let history: Vec<Placement> = latest.into_iter().map(|(_, p)| p).collect();
    (history.iter().filter(|p| p.placed).cloned().collect(), history)
}

/// The placements the habitat's head answers: those that hold, and every
/// one its record names.
pub(super) fn from_head() -> Result<(Vec<Placement>, Vec<Placement>), String> {
    let v = super::sapling::head_call("Commands::placements", json!({ "limit": 500 }))?;
    if v["ok"] != json!(true) {
        return Err(format!("the habitat's head answered no placements ({})", v["why"].as_str().unwrap_or("")));
    }
    Ok(parse_rows(&v))
}

/// The placements of a `Commands::placements` reply.
pub(super) fn parse_rows(v: &Value) -> (Vec<Placement>, Vec<Placement>) {
    let list = |k: &str| -> Vec<Placement> {
        v[k].as_str().and_then(|t| serde_json::from_str::<Vec<Value>>(t).ok()).unwrap_or_default().iter().map(Placement::of).collect()
    };
    (list("placements"), list("history"))
}

/// `hale habitat status`'s placements, under their hosts.
pub(super) fn status_lines(current: &[Placement]) -> Vec<String> {
    let mut out = vec!["placements, as the habitat's record:".to_string()];
    if current.is_empty() {
        out.push("placements: none (`hale habitat organism place` places an organism's host command)".to_string());
        return out;
    }
    let hosts: BTreeSet<&str> = current.iter().map(|p| p.host.as_str()).collect();
    for h in hosts {
        out.push(format!("host {h}"));
        for p in current.iter().filter(|p| p.host == h) {
            let service = if p.service.is_empty() { "no service named".to_string() } else { format!("{} ({})", p.service, kind(&p.platform)) };
            out.push(format!("  {}'s {} {}  {service} on {h}  {}", p.organism, p.command, p.id, p.readings()));
        }
    }
    out
}

// ---------------------------------------------------------------
// taking them off
// ---------------------------------------------------------------

/// Where placing installed on a host, under the login's home.
const HOSTED: &str = ".local/share/hale/hosted";
const TOOLCHAINS: &str = ".local/share/hale/toolchains";

/// What placing installed on `host` that no placement still there uses:
/// the organisms' checkouts and the toolchains, from every placement the
/// record names there (`history`) less those that hold (`remaining`).
pub(super) fn leftovers(host: &str, history: &[Placement], remaining: &[Placement]) -> (Vec<String>, Vec<String>) {
    let held = |f: fn(&Placement) -> &str| -> BTreeSet<String> { remaining.iter().filter(|p| p.host == host && p.placed).map(|p| f(p).to_string()).collect() };
    let (used_o, used_t) = (held(|p| &p.organism), held(|p| &p.toolchain));
    let of = |f: fn(&Placement) -> &str, used: &BTreeSet<String>| -> Vec<String> {
        history.iter().filter(|p| p.host == host).map(|p| f(p).to_string()).filter(|x| !x.is_empty() && !used.contains(x) && word(x)).collect::<BTreeSet<_>>().into_iter().collect()
    };
    (of(|p| &p.organism, &used_o), of(|p| &p.toolchain, &used_t))
}

/// Whether `s` is one path segment a script may name.
fn word(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The script that removes `organisms`' checkouts and `toolchains` under
/// the login's home (through `sudo` when the login alone cannot: a command
/// run as root leaves root's files in the checkout), then the habitat's
/// directories there that are left empty. Its last line says what it
/// removed and what it left: `hale-node: removed <n> left <paths>`.
pub(super) fn leftovers_script(organisms: &[String], toolchains: &[String], sudo: &str) -> String {
    let paths: Vec<String> = organisms.iter().filter(|o| word(o)).map(|o| format!("\"$HOME/{HOSTED}/{o}\"")).chain(toolchains.iter().filter(|t| word(t)).map(|t| format!("\"$HOME/{TOOLCHAINS}/{t}\""))).collect();
    format!(
        r#"S="{sudo}"; n=0; left=
for d in {list}; do
  [ -e "$d" ] || continue
  if rm -rf "$d" 2>/dev/null || $S rm -rf "$d" 2>/dev/null; then n=$((n + 1)); else left="$left $d"; fi
done
for d in "$HOME/{HOSTED}" "$HOME/{TOOLCHAINS}" "$HOME/.local/share/hale/habitat"; do [ -d "$d" ] && rmdir "$d" 2>/dev/null; done
printf 'hale-node: removed %s left%s\n' "$n" "$left""#,
        list = if paths.is_empty() { "\"\"".to_string() } else { paths.join(" ") },
    )
}

/// What placing installed on `host` and nothing still uses, removed over
/// the person's own login, and the habitat's directories left empty there:
/// what was said of it ("" for nothing to say).
pub(super) fn tidy_host(host: &str, port: Option<u16>, organisms: &[String], toolchains: &[String]) -> String {
    let what = organisms.iter().map(|o| format!("~/{HOSTED}/{o}")).chain(toolchains.iter().map(|t| format!("~/{TOOLCHAINS}/{t}"))).collect::<Vec<_>>();
    match super::remote::login_script(host, None, port, |sudo| leftovers_script(organisms, toolchains, sudo)) {
        Ok(o) => {
            let left = o.split_once(" left").map(|(_, l)| l.trim().to_string()).unwrap_or_default();
            if !left.is_empty() {
                format!("{:<10}on {host}: {left} (remove it there, as root where it is root's)", "left")
            } else if what.is_empty() {
                String::new()
            } else {
                format!("{:<10}on {host}: {}, what placing installed there", "removed", what.join(", "))
            }
        }
        Err(why) if !what.is_empty() => format!("{:<10}on {host}: {} ({why})", "left", what.join(", ")),
        Err(_) => String::new(),
    }
}

/// One placement's own `uninstall`, run as `unplace` runs it (which says
/// what it does as it does it, and records `habitat.unplaced`).
pub(super) fn uninstall(p: &Placement) -> Result<(), String> {
    let args: Vec<String> = [p.organism.as_str(), p.command.as_str(), "--host", p.host.as_str(), "--id", p.id.as_str()].iter().map(|s| s.to_string()).collect();
    if super::hosted::place(&args, false) == std::process::ExitCode::SUCCESS {
        Ok(())
    } else {
        Err("its uninstall did not finish (said above)".to_string())
    }
}

/// The plan's lines for taking `placed` off: each one's uninstall, then
/// what placing installed on its host.
pub(super) fn plan_lines(placed: &[Placement]) -> Vec<String> {
    let mut out = Vec::new();
    for p in placed {
        out.push(format!("{}'s `{}` {} taken off {} by its uninstall, over your SSH login ({}, {})", p.organism, p.command, p.id, p.host, if p.service.is_empty() { "no service named".to_string() } else { p.service.clone() }, p.readings()));
    }
    let hosts: BTreeSet<&str> = placed.iter().map(|p| p.host.as_str()).collect();
    for h in hosts {
        let orgs: BTreeSet<&str> = placed.iter().filter(|p| p.host == h).map(|p| p.organism.as_str()).collect();
        let tags: BTreeSet<&str> = placed.iter().filter(|p| p.host == h && !p.toolchain.is_empty()).map(|p| p.toolchain.as_str()).collect();
        out.push(format!(
            "then on {h}, over your SSH login, what placing installed there that no placement left uses: {}{}",
            orgs.iter().map(|o| format!("~/{HOSTED}/{o}")).collect::<Vec<_>>().join(", "),
            if tags.is_empty() { String::new() } else { format!(", the toolchain {}", tags.iter().map(|t| format!("~/{TOOLCHAINS}/{t}")).collect::<Vec<_>>().join(", ")) }
        ));
    }
    out
}

/// The hosts' SSH ports, as the habitat's head names them.
fn ports() -> Vec<(String, Option<u16>)> {
    super::sapling::head_call("Commands::hosts", json!({ "limit": 500 }))
        .ok()
        .and_then(|v| v["hosts"].as_str().and_then(|t| serde_json::from_str::<Vec<Value>>(t).ok()))
        .unwrap_or_default()
        .iter()
        .map(|h| (h["name"].as_str().unwrap_or("").to_string(), h["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0)))
        .collect()
}

/// The SSH port of `host`, as the habitat's head names it.
pub(super) fn port_of(host: &str) -> Option<u16> {
    ports().into_iter().find(|(h, _)| h == host).and_then(|(_, p)| p)
}

/// `placed` taken off, each by its own uninstall over the person's login,
/// then what placing installed on their hosts that no placement left there
/// uses (`history`: every placement the record names): a line for each. A
/// placement that cannot be taken off is said, with its `unplace`, and the
/// rest goes on.
pub(super) fn take_off(placed: &[Placement], history: &[Placement]) -> Vec<String> {
    let mut out = Vec::new();
    let ports = ports();
    for p in placed {
        match uninstall(p) {
            Ok(()) => out.push(format!("{:<10}{}'s `{}` {} taken off {}", "done", p.organism, p.command, p.id, p.host)),
            Err(why) => out.push(format!("{:<10}{}'s `{}` {} on {} ({}): {why}; `{}` takes it off, at a terminal (its sudo asks there)", "left", p.organism, p.command, p.id, p.host, if p.service.is_empty() { "no service named" } else { &p.service }, p.unplace_command())),
        }
    }
    // what still holds after them, as the record says now
    let remaining = from_head().map(|(c, _)| c).unwrap_or_else(|_| placed.to_vec());
    let hosts: BTreeSet<&str> = placed.iter().map(|p| p.host.as_str()).collect();
    for h in hosts {
        let (orgs, tags) = leftovers(h, history, &remaining);
        let port = ports.iter().find(|(n, _)| n == h).and_then(|(_, p)| *p);
        let said = tidy_host(h, port, &orgs, &tags);
        if !said.is_empty() {
            out.push(said);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, body: Value) -> Value {
        let p = Placement::of(&body);
        json!({ "kind": kind, "entity": p.key(), "body": body.to_string() })
    }

    fn body(org: &str, host: &str, id: &str, tag: &str, target: &str) -> Value {
        json!({ "organism": org, "command": "node", "host": host, "id": id, "service": format!("{org}-node-{id}"), "platform": "linux", "readings": 9464, "toolchain": tag, "target": target, "state": "placed" })
    }

    #[test]
    fn the_readings_key_is_a_port() {
        let t = |v: &str| -> toml::Table { toml::from_str(&format!("readings = {v}")).unwrap() };
        assert!(check_readings("node", &toml::Table::new()).is_ok(), "optional");
        assert!(check_readings("node", &t("9464")).is_ok());
        assert!(check_readings("node", &t("65535")).is_ok() && check_readings("node", &t("1")).is_ok());
        for bad in ["0", "65536", "-1", "\"9464\"", "9464.0", "true", "[9464]"] {
            let why = check_readings("node", &t(bad)).unwrap_err();
            assert!(why.contains("[dna.host.node] readings") && why.contains("1 to 65535"), "{bad}: {why}");
        }
        let m = "[dna]\nneeds = []\n\n[dna.host.node]\nreadings = 9464\n\n[dna.host.other]\ninstall = [\"x\"]\n";
        assert_eq!(readings_port(m, "node"), Some(9464));
        assert_eq!(readings_port(m, "other"), None);
        assert_eq!(readings_port(m, "missing"), None);
        assert_eq!(endpoint("gpu-1.lan:9464"), "http://gpu-1.lan:9464/metrics");
        // a host command's declaration takes it, and refuses one that is no port
        let decl = |r: &str| format!("[dna]\nneeds = []\n\n[dna.host.node]\ninstall = [\"hale\", \"run\", \"node\", \"install\"]\nuninstall = [\"hale\", \"run\", \"node\", \"uninstall\"]\nservice = \"node-{{id}}\"\n{r}\n");
        assert!(super::super::hosted::host_commands(&decl("readings = 9464")).is_ok());
        assert!(super::super::hosted::host_commands(&decl("")).is_ok());
        assert!(super::super::hosted::host_commands(&decl("readings = 70000")).unwrap_err().contains("readings"));
        assert!(super::super::hosted::host_commands(&decl("readings = \"9464\"")).unwrap_err().contains("readings"));
    }

    #[test]
    fn the_latest_row_of_each_placement_holds() {
        let rows = vec![
            row("habitat.placed", body("voice", "gpu-1", "gpu-1", "v0.30.1", "gpu-1.lan:9464")),
            row("habitat.placed", body("voice", "mac-1", "mac-1", "v0.30.1", "")),
            json!({ "kind": "habitat.host", "entity": "gpu-1", "body": "{}" }),
            row("habitat.unplaced", json!({ "organism": "voice", "command": "node", "host": "mac-1", "id": "mac-1", "toolchain": "v0.30.1", "state": "unplaced" })),
            // placed again, at a newer toolchain: the newer row is it
            row("habitat.placed", body("voice", "gpu-1", "gpu-1", "v0.31.0", "gpu-1.lan:9464")),
        ];
        let (current, history) = fold(&rows);
        assert_eq!(current.len(), 1, "{current:?}");
        assert_eq!((current[0].host.as_str(), current[0].toolchain.as_str()), ("gpu-1", "v0.31.0"));
        assert_eq!(current[0].readings(), "http://gpu-1.lan:9464/metrics");
        assert_eq!(history.len(), 2);
        assert!(history.iter().any(|p| p.host == "mac-1" && !p.placed));
        assert_eq!(current[0].unplace_command(), "hale habitat organism unplace voice node --host gpu-1 --id gpu-1");
        let lines = status_lines(&current);
        assert_eq!(lines[1], "host gpu-1");
        assert_eq!(lines[2], "  voice's node gpu-1  voice-node-gpu-1 (systemd) on gpu-1  http://gpu-1.lan:9464/metrics");
        assert!(status_lines(&[]).iter().any(|l| l.starts_with("placements: none")));
        let mut mac = body("voice", "mac-1", "mac-1", "v0.30.1", "");
        mac["platform"] = json!("macos");
        assert!(Placement::of(&mac).line().contains("(launchd)") && Placement::of(&mac).line().ends_with("no readings"));
    }

    #[test]
    fn what_placing_installed_goes_when_nothing_left_uses_it() {
        let p = |org: &str, host: &str, id: &str, tag: &str, placed: bool| Placement { placed, ..Placement::of(&body(org, host, id, tag, "")) };
        let history = vec![p("voice", "gpu-1", "a", "v0.30.1", true), p("voice", "gpu-1", "b", "v0.31.0", true), p("tts", "gpu-1", "c", "v0.30.1", false), p("voice", "mac-1", "m", "v0.30.1", true)];
        // all of gpu-1's taken off: both checkouts and both toolchains go
        assert_eq!(leftovers("gpu-1", &history, &[]), (vec!["tts".to_string(), "voice".to_string()], vec!["v0.30.1".to_string(), "v0.31.0".to_string()]));
        // one left there (its uninstall failed): what it uses stays
        let remaining = vec![history[1].clone()];
        assert_eq!(leftovers("gpu-1", &history, &remaining), (vec!["tts".to_string()], vec!["v0.30.1".to_string()]));
        // another host's placements are its own
        assert_eq!(leftovers("mac-1", &history, &remaining).0, ["voice"]);
        let s = leftovers_script(&["voice".to_string()], &["v0.30.1".to_string()], "sudo -n");
        assert!(s.contains("\"$HOME/.local/share/hale/hosted/voice\" \"$HOME/.local/share/hale/toolchains/v0.30.1\""), "{s}");
        assert!(s.contains("rmdir \"$d\""), "only an empty directory: {s}");
        for script in [s, leftovers_script(&[], &[], "sudo")] {
            assert!(std::process::Command::new("sh").args(["-n", "-c", &script]).status().unwrap().success(), "{script}");
        }
        // a name that is no path segment is never in a script
        assert!(!leftovers_script(&["../x".to_string()], &[], "sudo").contains("../x"));
        let plan = plan_lines(&history[..2]);
        assert!(plan[0].starts_with("voice's `node` a taken off gpu-1 by its uninstall"), "{plan:?}");
        assert!(plan[2].contains("~/.local/share/hale/hosted/voice") && plan[2].contains("~/.local/share/hale/toolchains/v0.30.1, ~/.local/share/hale/toolchains/v0.31.0"), "{plan:?}");
    }

    #[test]
    fn the_leftovers_script_removes_only_what_it_names_and_empty_directories() {
        let home = std::env::temp_dir().join(format!("hale-placements-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in [".local/share/hale/hosted/voice/.git", ".local/share/hale/hosted/kept", ".local/share/hale/toolchains/v0.30.1/bin", ".local/share/hale/habitat"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let out = std::process::Command::new("sh").args(["-c", &leftovers_script(&["voice".to_string()], &["v0.30.1".to_string()], "false")]).env("HOME", &home).output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert_eq!(said.trim(), "hale-node: removed 2 left", "{said}");
        assert!(!home.join(".local/share/hale/hosted/voice").exists() && home.join(".local/share/hale/hosted/kept").is_dir());
        assert!(!home.join(".local/share/hale/toolchains").exists(), "empty: removed");
        assert!(!home.join(".local/share/hale/habitat").exists(), "the habitat's empty directory goes too");
        std::fs::remove_dir_all(&home).ok();
    }
}
