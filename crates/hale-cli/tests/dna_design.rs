//! GH #596 C — the design as proposals. `init` seeds the toolchain's
//! practices about how a DNA organization works as `knowledge.proposed`
//! rows, one Board Review each: a Review pins one digest and settles
//! with one outcome, so the Board decides practice by practice. Nothing
//! is ratified by the toolchain. Supersession is a Board decision too:
//! ratifying a proposal that supersedes an earlier version retires it;
//! declining the replacement leaves the earlier version in force.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .output()
        .expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn hale_env(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale")).env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// The `supersedes` a proposal's receipt names ("" when none).
fn supersedes_of(app: &Path, digest: &str) -> String {
    let raw = digest.strip_prefix("sha256:").unwrap_or(digest);
    let out = Command::new("git").args(["cat-file", "-p", &format!("refs/dna/receipts/{raw}")]).current_dir(app).output().unwrap();
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap_or(serde_json::Value::Null);
    v["supersedes"].as_str().unwrap_or("").to_string()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn http(port: u16, req: &str) -> String {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = s.write_all(req.as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn body(resp: &str) -> String {
    resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
}

fn journal(app: &Path) -> Vec<(String, String, String)> {
    let out = Command::new("git").args(["show", "refs/dna/journal:journal.jsonl"]).current_dir(app).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (v["kind"].as_str().unwrap().to_string(), v["entity"].as_str().unwrap().to_string(), v["body"].as_str().unwrap_or("").to_string())
        })
        .collect()
}

/// The organization, running, until `finish`.
fn start_org(app: &Path) -> std::process::Child {
    // a previous organization's sockets would answer the wait below
    // before the new one has bound
    for s in ["hale-dna.intent.offered.sock", "hale-dna.review.verdict.sock"] {
        let _ = std::fs::remove_file(app.join(".hale/dna").join(s));
    }
    let host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl && !(app.join(".hale/dna/hale-dna.intent.offered.sock").exists() && app.join(".hale/dna/hale-dna.review.verdict.sock").exists()) {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(app.join(".hale/dna/hale-dna.review.verdict.sock").exists(), "the organization never bound its membrane");
    host
}

fn finish(app: &Path, host: &mut std::process::Child) {
    if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
    let _ = host.kill();
    let _ = host.wait();
}

/// The package for `org` from a fresh knowledge service over the record:
/// the digests included, and the raw body.
fn package(app: &Path) -> (Vec<String>, String) {
    let port = free_port();
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "knowledge", ".", "--port", &port.to_string()])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna knowledge");
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(200));
    }
    let ctx = body(&http(port, "GET /context?target=org&budget=32 HTTP/1.0\r\nHost: x\r\n\r\n"));
    let _ = c.kill();
    let _ = c.wait();
    let v: serde_json::Value = serde_json::from_str(&ctx).unwrap_or(serde_json::Value::Null);
    // `included` is the space-separated digests, in ratification order
    let ids: Vec<String> = v["included"].as_str().unwrap_or("").split_whitespace().map(|s| s.to_string()).collect();
    (ids, ctx)
}

/// A driver that ratifies an EARLIER version of two design practices
/// in-process: what a record looks like after a previous toolchain.
const OLDER: &str = r#"import "vendor/dna" as dna;

main locus App {
    params {
        core: dna::Dna = dna::Dna {
            journal: dna::GitJournal { repo: "." },
            verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, repo: "." },
            membrane: dna::Board { who: "board" }
        };
    }
    bus { publish dna::ReviewVerdict; }
    run() {
        let a = self.core.propose_knowledge(dna::Idea { id: "old-a", kind: "practice", text: "An earlier statement of the principles.", author: "org", name: "design/principles" }, "org");
        dna::ReviewVerdict <- dna::Verdict { review_id: dna::knowledge_review_id(a), subject_digest: a, verdict: "approve", reviewer: "riley", authority: "board" };
        std::time::sleep(400ms);
        let b = self.core.propose_knowledge(dna::Idea { id: "old-b", kind: "practice", text: "An earlier statement of evolution.", author: "org", name: "design/evolution" }, "org");
        dna::ReviewVerdict <- dna::Verdict { review_id: dna::knowledge_review_id(b), subject_digest: b, verdict: "approve", reviewer: "riley", authority: "board" };
        std::time::sleep(400ms);
        println(a);
        println(b);
    }
}

fn main() { App { }; }
"#;

#[test]
fn the_design_is_decided_practice_by_practice_and_superseded_by_the_board() {
    let d = std::env::temp_dir().join(format!("hale_dna_design_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "designed"], &d);
    assert!(ok, "{out}");
    assert!(out.contains("charter.hl") && out.contains("seeded  design (8 practice(s) proposed"), "{out}");
    let app: PathBuf = d.join("designed");
    assert!(app.join("dna/org/charter.hl").is_file(), "the charter is written beside the purpose");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();

    // one Review per practice, listed under one heading
    let (ok, list) = hale(&["dna", "review"], &app);
    assert!(ok, "{list}");
    assert!(list.contains("design — 8 seeded practice(s), each its own Review"), "{list}");
    let ids: Vec<String> = list.lines().filter_map(|l| l.trim().strip_prefix("k:").map(|r| format!("k:{}", &r[..12]))).collect();
    assert_eq!(ids.len(), 8, "{list}");
    let rows = journal(&app);
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.proposed").count(), 8, "eight proposals");
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.ratified").count(), 0, "nothing ratified by the toolchain");
    let digest_of = |id: &str| -> String {
        journal(&app).iter().find(|r| r.0 == "review.requested" && r.1 == format!("review:{id}")).map(|r| serde_json::from_str::<serde_json::Value>(&r.2).unwrap()["knowledge_digest"].as_str().unwrap().to_string()).unwrap_or_else(|| panic!("no review.requested for {id}"))
    };

    // the Board approves two and rejects the rest, one verdict each
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &ids[0], "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    let (ok, b) = hale(&["dna", "review", &ids[1], "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && b.contains("settled: approve by riley"), "{b}");
    let (ok, rest) = hale(&["dna", "review", "design", "reject", "--as", "riley", "--authority", "board"], &app);
    assert!(ok, "{rest}");
    assert_eq!(rest.matches("settled: reject by riley").count(), 6, "the six still pending, each its own verdict:\n{rest}");
    finish(&app, &mut host);
    let rows = journal(&app);
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.ratified").count(), 2, "two ratified");
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.declined").count(), 6, "six declined");
    // the package holds exactly the two, through a fresh service (a replay of the record)
    let (included, ctx) = package(&app);
    let (d0, d1) = (digest_of(&ids[0]), digest_of(&ids[1]));
    assert!(included.contains(&d0) && included.contains(&d1), "the approved practices are in the package: {ctx}");
    assert_eq!(included.len(), 2, "and nothing else: {ctx}");

    // ---- supersession: an earlier version in the record, then `upgrade`
    std::fs::create_dir_all(app.join("older")).unwrap();
    std::fs::write(app.join("older/main.hl"), OLDER).unwrap();
    let (ok, out) = hale(&["run", "older"], &app);
    assert!(ok, "older: {out}");
    let mut olds = out.lines().filter(|l| l.starts_with("sha256:")).map(|s| s.trim().to_string());
    let old_principles = olds.next().expect("old principles digest");
    let old_evolution = olds.next().expect("old evolution digest");
    let (included, ctx) = package(&app);
    assert!(included.contains(&old_principles) && included.contains(&old_evolution), "the earlier versions are in force: {ctx}");
    // upgrade proposes the current text of each, superseding the earlier one
    let (ok, up) = hale(&["dna", "upgrade"], &app);
    assert!(ok, "{up}");
    assert!(up.contains("design  2 practice(s) proposed (2 superseding an earlier version)"), "{up}");
    let (ok, up2) = hale(&["dna", "upgrade"], &app);
    assert!(ok && !up2.contains("design  "), "a second upgrade proposes nothing: {up2}");
    let rows = journal(&app);
    let superseding: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r.0 == "review.requested")
        .filter_map(|r| {
            let b: serde_json::Value = serde_json::from_str(&r.2).ok()?;
            if b["group"] != "design" { return None; }
            let id = r.1.strip_prefix("review:")?.to_string();
            let name = b["name"].as_str()?.to_string();
            Some((id, name))
        })
        .filter(|(id, _)| !ids.contains(id))
        .collect();
    assert_eq!(superseding.len(), 2, "{superseding:?}");
    let new_principles = superseding.iter().find(|(_, n)| n == "design/principles").map(|(i, _)| i.clone()).unwrap();
    let new_evolution = superseding.iter().find(|(_, n)| n == "design/evolution").map(|(i, _)| i.clone()).unwrap();
    // the Board takes the new principles and keeps the old evolution
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &new_principles, "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    let (ok, b) = hale(&["dna", "review", &new_evolution, "reject", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && b.contains("settled: reject by riley"), "{b}");
    finish(&app, &mut host);
    let rows = journal(&app);
    let retired: Vec<&(String, String, String)> = rows.iter().filter(|r| r.0 == "knowledge.retired").collect();
    assert_eq!(retired.len(), 1, "one retirement, by the ratified replacement: {retired:?}");
    assert_eq!(retired[0].1, old_principles, "the old principles were retired");
    let (included, ctx) = package(&app);
    assert!(!included.contains(&old_principles), "the retired version left the package: {ctx}");
    assert!(included.contains(&old_evolution), "a declined replacement leaves the earlier version in force: {ctx}");
    let new_p = digest_of(&new_principles);
    assert!(included.contains(&new_p), "the ratified replacement is in the package: {ctx}");
    assert_eq!(included.len(), 4, "two from the first round, the old evolution, the new principles: {ctx}");

    // ---- a rejected replacement is not a predecessor. The record now
    // holds, for evolution: the old version ACTIVE and the toolchain's
    // replacement REJECTED. A later toolchain (its text changed: the
    // suffix knob stands in for it) must supersede the active old
    // version, not the never-active rejected one — a review found the
    // replacement naming the rejected one, retiring nothing, and the
    // package serving old and new together.
    let (ok, up3) = hale_env(&["dna", "upgrade"], &app, &[("HALE_DNA_DESIGN_SUFFIX", " (a later toolchain)")]);
    assert!(ok, "{up3}");
    // every text changed, so eight are proposed; only four names have
    // something ACTIVE to supersede (the two approved in the first
    // round, the ratified principles, the old evolution) — the six
    // rejected in the first round have nothing to retire
    assert!(up3.contains("design  8 practice(s) proposed (4 superseding an earlier version)"), "every practice changed; those with an active predecessor supersede it: {up3}");
    let rows = journal(&app);
    let later: Vec<(String, String, String)> = rows
        .iter()
        .filter(|r| r.0 == "review.requested")
        .filter_map(|r| {
            let b: serde_json::Value = serde_json::from_str(&r.2).ok()?;
            if b["group"] != "design" { return None; }
            let id = r.1.strip_prefix("review:")?.to_string();
            // the third round's Reviews: neither the seeded nor the first upgrade's
            if ids.contains(&id) || superseding.iter().any(|(s, _)| s == &id) { return None; }
            Some((id, b["name"].as_str()?.to_string(), b["knowledge_digest"].as_str()?.to_string()))
        })
        .collect();
    assert_eq!(later.len(), 8, "{later:?}");
    let (later_evolution, later_evolution_digest) = later.iter().find(|(_, n, _)| n == "design/evolution").map(|(i, _, dg)| (i.clone(), dg.clone())).unwrap();
    let (later_principles, later_principles_digest) = later.iter().find(|(_, n, _)| n == "design/principles").map(|(i, _, dg)| (i.clone(), dg.clone())).unwrap();
    assert_eq!(supersedes_of(&app, &later_evolution_digest), old_evolution, "the replacement of evolution supersedes the ACTIVE old version, not the rejected replacement");
    assert_eq!(supersedes_of(&app, &later_principles_digest), new_p, "and the replacement of principles supersedes the ratified replacement, which is what is active there");
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &later_evolution, "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    finish(&app, &mut host);
    let rows = journal(&app);
    let retired: Vec<String> = rows.iter().filter(|r| r.0 == "knowledge.retired").map(|r| r.1.clone()).collect();
    assert!(retired.contains(&old_evolution), "the active old evolution was retired: {retired:?}");
    let (included, ctx) = package(&app);
    assert!(included.contains(&later_evolution_digest), "the later evolution is in the package: {ctx}");
    assert!(!included.contains(&old_evolution) && !included.contains(&digest_of(&new_evolution)), "neither the retired old version nor the rejected replacement is: {ctx}");
    assert_eq!(included.len(), 4, "two from the first round, the ratified principles, the later evolution: {ctx}");

    // ---- one replacement at a time. Seven of the third round's
    // proposals are still before the Board (principles among them,
    // superseding the ratified principles). A yet later toolchain
    // changes every text again: evolution, whose latest proposal was
    // decided, is proposed superseding it; the seven wait — a second
    // pending replacement would name the same predecessor, and the
    // assembly refuses to ratify it once the first has retired that
    // (a review found both served, the first never retired).
    let (ok, up4) = hale_env(&["dna", "upgrade"], &app, &[("HALE_DNA_DESIGN_SUFFIX", " (a yet later toolchain)")]);
    assert!(ok, "{up4}");
    assert!(up4.contains("design  1 practice(s) proposed (1 superseding an earlier version)"), "only evolution, whose latest proposal is decided: {up4}");
    assert!(up4.contains("design  7 practice(s) changed but wait: an earlier replacement is still before the Board"), "the rest wait: {up4}");
    let proposals = |app: &Path| journal(app).iter().filter(|r| r.0 == "knowledge.proposed").count();
    let n4 = proposals(&app);
    let (ok, up4b) = hale_env(&["dna", "upgrade"], &app, &[("HALE_DNA_DESIGN_SUFFIX", " (a yet later toolchain)")]);
    assert!(ok && !up4b.contains("practice(s) proposed") && up4b.contains("7 practice(s) changed but wait"), "again proposes nothing more: {up4b}");
    assert_eq!(proposals(&app), n4, "and the record grew by nothing");
    // the Board decides the pending principles; the next upgrade
    // proposes the yet later principles against what is active NOW
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &later_principles, "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    finish(&app, &mut host);
    let (ok, up5) = hale_env(&["dna", "upgrade"], &app, &[("HALE_DNA_DESIGN_SUFFIX", " (a yet later toolchain)")]);
    assert!(ok, "{up5}");
    assert!(up5.contains("design  1 practice(s) proposed (1 superseding an earlier version)") && up5.contains("6 practice(s) changed but wait"), "{up5}");
    let rows = journal(&app);
    let yet_principles = rows
        .iter()
        .filter(|r| r.0 == "review.requested")
        .filter_map(|r| {
            let b: serde_json::Value = serde_json::from_str(&r.2).ok()?;
            if b["group"] != "design" || b["name"] != "design/principles" { return None; }
            Some(b["knowledge_digest"].as_str()?.to_string())
        })
        .last()
        .unwrap();
    assert_ne!(yet_principles, later_principles_digest);
    assert_eq!(supersedes_of(&app, &yet_principles), later_principles_digest, "the yet later principles supersede the version the Board just ratified");
    let _ = std::fs::remove_dir_all(&d);
}
