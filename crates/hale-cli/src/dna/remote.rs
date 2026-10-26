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

use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use super::{default_habitat_image_tag, face_state_dir, head_url_file, http_surface_call, manifest_pin_in_text, usage_text, HABITAT_PIN};
use crate::dna::local_habitat_dir;

/// The pointer's file, in the local habitat's directory.
const POINTER: &str = "remote.json";

/// The variables a container on the habitat's machine is told its address
/// by: the address its person-facing listeners bind (`LISTEN`) and the
/// name the habitat is reached by (`ADDRESS`).
pub(super) const LISTEN_ENV: &str = "HALE_DNA_HABITAT_LISTEN";
pub(super) const ADDRESS_ENV: &str = "HALE_DNA_HABITAT_ADDRESS";

/// What the person's environment carries to the containers on the
/// habitat's machine, as a local `up` hands it to its body and face.
const FORWARDED: &[&str] = &[
    "HALE_DNA_DISCOVER",
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

/// A word for the remote shell, single-quoted.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// What the machine says of itself: its user's home, uid, gid, name and the
/// docker socket's group.
struct Machine {
    home: String,
    uid: String,
    gid: String,
    user: String,
    sock_gid: String,
}

fn probe(t: &Target) -> Result<Machine, String> {
    let said = t.run("printf '%s\\n' \"$HOME\"; id -u; id -g; id -un; stat -c %g /var/run/docker.sock 2>/dev/null || echo 0; docker version --format '{{.Server.Version}}' >/dev/null 2>&1 && echo docker")?;
    let lines: Vec<&str> = said.lines().map(str::trim).collect();
    if lines.len() < 6 || lines[5] != "docker" {
        return Err(format!("{} answers over SSH, but its docker does not (the habitat's machine needs docker and the user's access to its socket)", t.host));
    }
    if lines[0].is_empty() || !lines[0].starts_with('/') {
        return Err(format!("{} names no home directory for its user", t.host));
    }
    Ok(Machine { home: lines[0].to_string(), uid: lines[1].to_string(), gid: lines[2].to_string(), user: lines[3].to_string(), sock_gid: lines[4].to_string() })
}

/// The habitat's directory on its machine and what holds it (the user's
/// data directory, mounted into every container run there).
fn data_dir(m: &Machine) -> String {
    format!("{}/.local/share/hale", m.home)
}

fn habitat_dir(m: &Machine) -> String {
    format!("{}/habitat/local", data_dir(m))
}

/// The address the machine is reached at from here, as this machine
/// resolves its name: the address its person-facing listeners bind there.
fn listen_address(host: &str) -> Result<String, String> {
    use std::net::ToSocketAddrs;
    let addrs: Vec<std::net::SocketAddr> = (host, 0u16).to_socket_addrs().map_err(|e| format!("{host} does not resolve here: {e}"))?.collect();
    addrs.iter().find(|a| a.is_ipv4()).or_else(|| addrs.first()).map(|a| a.ip().to_string()).ok_or_else(|| format!("{host} resolves to no address here"))
}

/// The toolchain image on the machine: there already, sent from this
/// machine's docker (`docker save | docker load`), or pulled from the
/// published images when the one of this toolchain's version answers as
/// this toolchain does. Nothing is compiled there.
fn ensure_image(t: &Target, tag: &str) -> Result<String, String> {
    if t.run(&format!("docker image inspect {} >/dev/null 2>&1 && echo yes || true", sq(tag)))?.trim() == "yes" {
        return Ok(format!("{:<8}{tag} (on {} already)", "image", t.host));
    }
    let here = Command::new("docker").args(["image", "inspect", tag]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if here {
        super::progress(&format!("sending the image {tag} to {}", t.host));
        let mut save = Command::new("docker").args(["save", tag]).stdout(Stdio::piped()).spawn().map_err(|e| format!("docker save: {e}"))?;
        let stdout = save.stdout.take().ok_or("docker save: no output")?;
        let loaded = t.ssh(false).arg("docker load -q").stdin(stdout).stdout(Stdio::null()).status().map_err(|e| format!("ssh: {e}"))?;
        let saved = save.wait().map_err(|e| format!("docker save: {e}"))?;
        if !saved.success() || !loaded.success() {
            return Err(format!("the image {tag} could not be sent to {}", t.host));
        }
        return Ok(format!("{:<8}{tag} (sent from this machine)", "image"));
    }
    let published = format!("ghcr.io/hale-lang/hale:{}", env!("CARGO_PKG_VERSION"));
    super::progress(&format!("pulling {published} on {}", t.host));
    let ours = Command::new(std::env::current_exe().map_err(|e| e.to_string())?).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let theirs = t.run(&format!("docker pull -q {p} >/dev/null 2>&1 && docker run --rm --entrypoint hale {p} --version", p = sq(&published))).map(|s| s.trim().to_string()).unwrap_or_default();
    if theirs.is_empty() || theirs != ours {
        return Err(format!(
            "no image of this toolchain reaches {}: {tag} is neither there nor in this machine's docker, and {published} {} (`hale habitat image build` builds it here, `--image <tag>` names another)",
            t.host,
            if theirs.is_empty() { "could not be pulled there".to_string() } else { format!("is another build ({})", theirs.lines().last().unwrap_or("")) }
        ));
    }
    t.run(&format!("docker tag {} {}", sq(&published), sq(tag)))?;
    Ok(format!("{:<8}{tag} (pulled there as {published})", "image"))
}

/// The `docker run` on the machine that runs `hale habitat <args>` in the
/// toolchain image as its user, with its data directory and the docker
/// socket, on the host's network: the same `hale` a local habitat runs.
fn in_image(m: &Machine, p: &serde_json::Value, args: &[&str], tty: bool) -> String {
    let data = data_dir(m);
    let image = p["image"].as_str().unwrap_or("");
    let mut env = vec![
        ("HOME", m.home.clone()),
        ("USER", m.user.clone()),
        ("XDG_DATA_HOME", format!("{}/.local/share", m.home)),
        ("HALE_DNA_HABITAT", habitat_dir(m)),
        ("HALE_VAULT_DIR", format!("{data}/vault")),
        ("HALE_HABITAT_IMAGE", image.to_string()),
        (LISTEN_ENV, p["listen"].as_str().unwrap_or("127.0.0.1").to_string()),
        (ADDRESS_ENV, p["host"].as_str().unwrap_or("").to_string()),
    ];
    for k in FORWARDED {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                env.push((k, v));
            }
        }
    }
    let mut line = format!(
        "mkdir -p {d} && exec docker run --rm {it}--network host --user {u}:{g} --group-add {sg} -v {d}:{d} -v /var/run/docker.sock:/var/run/docker.sock -w {h}",
        d = sq(&data),
        it = if tty { "-i -t " } else { "" },
        u = sq(&m.uid),
        g = sq(&m.gid),
        sg = sq(&m.sock_gid),
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
    let listen = listen_address(host)?;
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
    out.push(format!("{:<8}{}… ({})", "record", &identity[..12.min(identity.len())], if kept.is_some() { "reconnected; the pointer is the same habitat's" } else { "pointed at from this machine" }));
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
    println!("{:<8}{}/ ({})", "face", p["face"].as_str().unwrap_or(""), if answers(p["face"].as_str().unwrap_or("")) { "up" } else { "down: `hale habitat up --host` starts it" });
    println!("{:<8}{head} ({})", "head", if up { "up" } else { "down" });
    if !up {
        return ExitCode::SUCCESS;
    }
    match http_surface_call(head, "Commands::index", &serde_json::json!({ "limit": 200 })) {
        Ok(v) if v["ok"] == serde_json::json!(true) && v["name"].as_str().is_some_and(|n| n != p["name"].as_str().unwrap_or("")) => {
            println!("{:<8}the head answers as the habitat `{}`, not this pointer's", "note", v["name"].as_str().unwrap_or(""));
        }
        Ok(_) => {}
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
                println!("  {:<24} {:<8} {} on {}: {}{}", f("allocation"), f("role"), f("provider"), if f("applied").is_empty() { f("host") } else { f("applied") }, f("state"), if f("detail").is_empty() { String::new() } else { format!(" ({})", f("detail")) });
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
    if confirm {
        // gone there whole: the pointer goes last
        match remote_state(&t, &m) {
            Ok((_, _, identity)) if identity.is_empty() => {
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
    fn the_run_there_carries_its_address_and_its_data() {
        let m = Machine { home: "/home/h".into(), uid: "1000".into(), gid: "1000".into(), user: "h".into(), sock_gid: "998".into() };
        let p = serde_json::json!({ "image": "hale-habitat:abc", "listen": "192.168.1.20", "host": "rack-a.lan" });
        let line = in_image(&m, &p, &["up", "--name", "x"], false);
        assert!(line.starts_with("mkdir -p '/home/h/.local/share/hale' && exec docker run --rm --network host --user '1000':'1000' --group-add '998'"), "{line}");
        assert!(line.contains("-e 'HALE_DNA_HABITAT_LISTEN=192.168.1.20'") && line.contains("-e 'HALE_DNA_HABITAT_ADDRESS=rack-a.lan'"), "{line}");
        assert!(line.contains("-e 'HALE_VAULT_DIR=/home/h/.local/share/hale/vault'"), "{line}");
        assert!(line.ends_with("'hale-habitat:abc' habitat 'up' '--name' 'x'"), "{line}");
    }
}
