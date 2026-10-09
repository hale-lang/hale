//! GH #596 C — the design as proposals. `init` seeds the toolchain's
//! practices about how a DNA organization works as `knowledge.proposed`
//! rows, one Board Review each: a Review pins one digest and settles
//! with one outcome, so the Board decides practice by practice. Nothing
//! is ratified by the toolchain. Supersession is a Board decision too:
//! ratifying a proposal that supersedes an earlier version retires it;
//! declining the replacement leaves the earlier version in force.

#[path = "support/vault.rs"]
mod vault;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let out = vault::hale()
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
    let _s = trace::Span::new("hale", args.join(" "));
    let mut c = vault::hale();
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

fn journal(app: &Path) -> Vec<(String, String, String)> {
    let _s = trace::Span::new("git", "show refs/dna/journal:journal.jsonl");
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

/// A role's NATS URL or token for this record (`HALE_DNA_NATS_URL_SPINE=`
/// or `HALE_DNA_NATS_ORG=`), from the owner's migration (GH #986).
fn nerves_env(app: &Path, line: &str) -> String {
    let (ok, out) = hale(&["dna", "nerves", "migrate"], app);
    assert!(ok, "nerves migrates: {out}");
    out.lines().find_map(|l| l.strip_prefix(line)).unwrap_or_else(|| panic!("no {line} in: {out}")).to_string()
}

/// The organization, running, until `finish`.
fn start_org(app: &Path) -> std::process::Child {
    let spine = memory_dsn(app, "HALE_DNA_MEMORY_DSN_SPINE=");
    let nats_spine = nerves_env(app, "HALE_DNA_NATS_URL_SPINE=");
    let nats_org = nerves_env(app, "HALE_DNA_NATS_ORG=");
    let _s = trace::Span::new("start_org", "hale dna run");
    let log = app.join(".hale/dna/host.log");
    let _ = std::fs::remove_file(&log);
    let host = vault::hale()
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_MEMORY_DSN_SPINE", spine)
        .env("HALE_DNA_NATS_URL_SPINE", nats_spine)
        .env("HALE_DNA_NATS_ORG", nats_org)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("hale dna run");
    let up = || std::fs::read_to_string(&log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    trace::wait_until("dna run: the organization reads its facts from the nerves", Duration::from_secs(180), Duration::from_millis(200), up);
    assert!(up(), "the organization never read its facts from the nerves:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
    host
}

type Rows = [(String, String, String)];

/// How many rows of `kind` the record holds.
fn count_of(rows: &Rows, kind: &str) -> usize {
    rows.iter().filter(|r| r.0 == kind).count()
}

/// Whether the record holds a row of `kind` about `entity`.
fn has_row(rows: &Rows, kind: &str, entity: &str) -> bool {
    rows.iter().any(|r| r.0 == kind && r.1 == entity)
}

/// Stop the organism once the record holds what the round's verdicts
/// lead to. A verdict's "settled" answer is the Review's own row; what
/// the organism does ABOUT it — `knowledge.ratified` / `declined` /
/// `retired` — it appends after, in `on_review_settled`, each row one
/// more refresh and append of the record. Killing the organism on the
/// last answer raced those rows (GH #970), and so did waiting for the
/// record to stand still for two seconds: the gap between a
/// ratification and the retirement it implies is the organism's to
/// take, and a loaded machine takes longer. So each round names the
/// rows it expects and this waits for them — two minutes at most; the
/// assertions after it say what is missing.
fn finish(app: &Path, host: &mut std::process::Child, what: &str, until: impl Fn(&Rows) -> bool) {
    trace::wait_until(format!("the record holds {what}"), Duration::from_secs(120), Duration::from_millis(250), || until(&journal(app)));
    if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
    let _ = host.kill();
    let _ = host.wait();
}

/// Memory's owner, from the environment (CI's service container). The
/// package is read from memory, so without one this test has nothing to
/// look at.
fn owner_dsn() -> Option<String> {
    std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok().filter(|d| !d.is_empty())
}

/// The nerves' owner (GH #986), from the environment (CI's NATS
/// service). A verdict reaches the organization only over the nerves,
/// so without one this test has nothing to exercise either.
fn nats_owner_url() -> Option<String> {
    std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty())
}

/// A role's DSN for this record (`HALE_DNA_MEMORY_DSN_SPINE=` or `…_HEAD=`),
/// from the owner's migration (GH #985).
fn memory_dsn(app: &Path, line: &str) -> String {
    let (ok, out) = hale(&["dna", "memory", "migrate"], app);
    assert!(ok, "memory migrates: {out}");
    out.lines().find_map(|l| l.strip_prefix(line)).unwrap_or_else(|| panic!("no {line} in: {out}")).to_string()
}

/// A head's read of the package for `org`, as memory holds it.
const PACKAGE: &str = r#"import "vendor/dna" as dna;

fn main() {
    let k = dna::MemoryKnowledge { repo: ".", dsn_env: "HALE_DNA_MEMORY_DSN_HEAD", budget: 32 };
    let p = k.package_for("org", "");
    let j = dna::GitJournal { repo: "." };
    let moved = j.refresh();
    println(p.error + "\t" + to_string(p.revision) + "\t" + to_string(j.revision()) + "\t" + p.included);
}
"#;

/// The package for `org`, read as a head once the spine has projected
/// every row of the record: the organization runs until its projection
/// reaches the record's head, the head reads, the organization stops.
/// The digests included, and the raw answer.
fn package(app: &Path) -> (Vec<String>, String) {
    package_for(app, "org")
}

/// The same, for a set of targets (space-separated paths and node ids).
fn package_for(app: &Path, targets: &str) -> (Vec<String>, String) {
    let _s = trace::Span::new("package", "the spine projects; a head reads");
    std::fs::create_dir_all(app.join("pkg")).unwrap();
    std::fs::write(app.join("pkg/main.hl"), PACKAGE.replace("package_for(\"org\", ", &format!("package_for(\"{targets}\", "))).unwrap();
    let head = memory_dsn(app, "HALE_DNA_MEMORY_DSN_HEAD=");
    let mut host = start_org(app);
    let mut last = String::new();
    trace::wait_until("the projection reaches the record's head", Duration::from_secs(120), Duration::from_millis(250), || {
        let (ok, out) = hale_env(&["run", "pkg"], app, &[("HALE_DNA_MEMORY_DSN_HEAD", head.as_str())]);
        // the program's one line, not the build's: importing the core
        // prints the `pq` deferral warnings (spec/semantics.md § "@sealed")
        // on stderr, which comes after stdout here
        last = out.lines().rev().find(|l| l.split('\t').count() == 4).unwrap_or("").to_string();
        let f: Vec<&str> = last.split('\t').collect();
        ok && f.len() == 4 && f[0].is_empty() && f[1] == f[2]
    });
    if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
    let _ = host.kill();
    let _ = host.wait();
    let f: Vec<&str> = last.split('\t').collect();
    assert!(f.len() == 4 && f[0].is_empty() && f[1] == f[2], "the projection never reached the record's head: {last}");
    let ids: Vec<String> = f[3].split_whitespace().map(|s| s.to_string()).collect();
    (ids, last)
}

/// The review ids `hale dna review` lists under one family's heading.
fn family_ids(list: &str, family: &str) -> Vec<String> {
    let heading = format!("  {family} — ");
    list.lines()
        .skip_while(|l| !l.starts_with(&heading))
        .skip(1)
        .take_while(|l| l.starts_with("      k:"))
        .filter_map(|l| l.trim().strip_prefix("k:").map(|r| format!("k:{}", &r[..12])))
        .collect()
}

/// This record's schema, roles and stream, gone again.
fn unmigrate(app: &Path, owner: &str) {
    let out = Command::new("git").args(["rev-list", "--max-parents=0", "refs/dna/journal"]).current_dir(app).output().unwrap();
    let id = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
    let sch = format!("dna_{id}");
    let sql = format!("DROP SCHEMA IF EXISTS {sch} CASCADE; DROP ROLE IF EXISTS {sch}_spine; DROP ROLE IF EXISTS {sch}_head");
    let _ = Command::new("psql").args([owner, "-q", "-c", &sql]).output();
    // and its stream on the nerves (GH #986), with the owner's URL
    let _ = vault::hale().args(["dna", "nerves", "drop", "."]).current_dir(app).env("HALE_DNA_DISCOVER", "off").output();
}

/// A driver that ratifies an EARLIER version of two design practices
/// in-process: what a record looks like after a previous toolchain.
const OLDER: &str = r#"import "vendor/dna" as dna;

main locus App {
    params {
        core: dna::Dna = dna::Dna {
            journal: dna::GitJournal { repo: "." },
            verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, repo: "." },
            board: dna::Board { who: "board" }
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
    let _t = trace::test("dna_design");
    let Some(owner) = owner_dsn() else {
        eprintln!("dna_design: no HALE_DNA_MEMORY_DSN_OWNER; the package is memory's, so nothing was exercised");
        return;
    };
    let Some(_nats_owner) = nats_owner_url() else {
        eprintln!("dna_design: no HALE_DNA_NATS_URL_OWNER; a verdict cannot reach the organization, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_design_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "designed", "--no-library"], &d);
    assert!(ok, "{out}");
    assert!(out.contains("charter.hl") && out.contains("seeded  design (8 practice(s) proposed") && out.contains("seeded  operating (7 practice(s) proposed"), "{out}");
    let app: PathBuf = d.join("designed");
    assert!(app.join("dna/org/charter.hl").is_file(), "the charter is written beside the purpose");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();

    // one Review per practice, listed under one heading
    let (ok, list) = hale(&["dna", "review"], &app);
    assert!(ok, "{list}");
    assert!(list.contains("design — 8 seeded practice(s), each its own Review"), "{list}");
    let ids = family_ids(&list, "design");
    assert_eq!(ids.len(), 8, "{list}");
    // the operating family is its own set (GH #994), undecided throughout
    // this test: its practices stay proposals and never reach a package
    assert!(list.contains("operating — 7 seeded practice(s), each its own Review"), "{list}");
    assert_eq!(family_ids(&list, "operating").len(), 7, "{list}");
    let rows = journal(&app);
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.proposed").count(), 16, "sixteen proposals: the purpose (GH #995), eight design, seven operating");
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.ratified").count(), 0, "nothing ratified by the toolchain");
    let digest_of = |id: &str| -> String {
        journal(&app).iter().find(|r| r.0 == "review.requested" && r.1 == format!("review:{id}")).map(|r| serde_json::from_str::<serde_json::Value>(&r.2).unwrap()["knowledge_digest"].as_str().unwrap().to_string()).unwrap_or_else(|| panic!("no review.requested for {id}"))
    };

    // the Board approves two and rejects the rest, one verdict each.
    // The two are chosen by NAME, and are neither principles nor
    // evolution: the rounds below put older versions of those two into
    // the record and count what is active under each name. Review ids
    // are digests, and a receipt carries the toolchain version, so the
    // listing's order moves with every release — picking by position
    // approved principles at v0.20.0 and one active predecessor vanished.
    let review_of = |name: &str| -> String {
        journal(&app)
            .iter()
            .find(|r| r.0 == "review.requested" && serde_json::from_str::<serde_json::Value>(&r.2).map(|b| b["name"] == name).unwrap_or(false))
            .map(|r| r.1.strip_prefix("review:").unwrap().to_string())
            .unwrap()
    };
    let ids: Vec<String> = {
        let first = [review_of("design/signals"), review_of("design/optimize")];
        first.iter().cloned().chain(ids.into_iter().filter(|i| !first.contains(i))).collect()
    };
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &ids[0], "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    let (ok, b) = hale(&["dna", "review", &ids[1], "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && b.contains("settled: approve by riley"), "{b}");
    let (ok, rest) = hale(&["dna", "review", "design", "reject", "--as", "riley", "--authority", "board"], &app);
    assert!(ok, "{rest}");
    assert_eq!(rest.matches("settled: reject by riley").count(), 6, "the six still pending, each its own verdict:\n{rest}");
    finish(&app, &mut host, "two ratifications and six declines", |rows| count_of(rows, "knowledge.ratified") >= 2 && count_of(rows, "knowledge.declined") >= 6);
    let rows = journal(&app);
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.ratified").count(), 2, "two ratified");
    assert_eq!(rows.iter().filter(|r| r.0 == "knowledge.declined").count(), 6, "six declined");
    // the package holds exactly the two, as memory holds the record
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
    let (new_principles_digest, new_evolution_digest) = (digest_of(&new_principles), digest_of(&new_evolution));
    finish(&app, &mut host, "the new principles ratified, the old retired, the new evolution declined", |rows| {
        has_row(rows, "knowledge.ratified", &new_principles_digest) && has_row(rows, "knowledge.retired", &old_principles) && has_row(rows, "knowledge.declined", &new_evolution_digest)
    });
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
    finish(&app, &mut host, "the later evolution ratified and the old one retired", |rows| {
        has_row(rows, "knowledge.ratified", &later_evolution_digest) && has_row(rows, "knowledge.retired", &old_evolution)
    });
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
    finish(&app, &mut host, "the later principles ratified and the ratified principles retired", |rows| {
        has_row(rows, "knowledge.ratified", &later_principles_digest) && has_row(rows, "knowledge.retired", &new_p)
    });
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
    unmigrate(&app, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

/// GH #994 — the operating practices are a second seeded family, decided
/// and superseded the same way as the design: one Review per practice,
/// `hale dna review operating approve|reject` over the pending ones, and
/// an `upgrade` whose text changed supersedes only the active version.
#[test]
fn the_operating_practices_are_seeded_decided_and_superseded_like_the_design() {
    let _t = trace::test("dna_design::operating");
    let Some(owner) = owner_dsn() else {
        eprintln!("dna_design: no HALE_DNA_MEMORY_DSN_OWNER; the package is memory's, so nothing was exercised");
        return;
    };
    let Some(_nats_owner) = nats_owner_url() else {
        eprintln!("dna_design: no HALE_DNA_NATS_URL_OWNER; a verdict cannot reach the organization, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_operating_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "operated"], &d);
    assert!(ok && out.contains("seeded  operating (7 practice(s) proposed"), "{out}");
    let app: PathBuf = d.join("operated");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();

    let (ok, list) = hale(&["dna", "review"], &app);
    assert!(ok && family_ids(&list, "operating").len() == 7, "seven operating Reviews, under their own heading:\n{list}");
    let review_of = |name: &str| -> String {
        journal(&app)
            .iter()
            .filter(|r| r.0 == "review.requested" && serde_json::from_str::<serde_json::Value>(&r.2).map(|b| b["name"] == name).unwrap_or(false))
            .map(|r| r.1.strip_prefix("review:").unwrap().to_string())
            .last()
            .unwrap()
    };
    let digest_of = |id: &str| -> String {
        journal(&app).iter().find(|r| r.0 == "review.requested" && r.1 == format!("review:{id}")).map(|r| serde_json::from_str::<serde_json::Value>(&r.2).unwrap()["knowledge_digest"].as_str().unwrap().to_string()).unwrap()
    };

    // the Board takes one and declines the rest of the family
    let row_first = review_of("operating/row-first");
    let old = digest_of(&row_first);
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &row_first, "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    let (ok, rest) = hale(&["dna", "review", "operating", "reject", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && rest.matches("settled: reject by riley").count() == 6, "the six still pending, each its own verdict:\n{rest}");
    finish(&app, &mut host, "one operating practice ratified and six declined", |rows| has_row(rows, "knowledge.ratified", &old) && count_of(rows, "knowledge.declined") >= 6);
    let (included, ctx) = package(&app);
    assert_eq!(included, vec![old.clone()], "the ratified operating practice is the package, the design being undecided: {ctx}");

    // a later toolchain: every text changed; the ratified one is superseded,
    // the declined ones are proposed afresh, the pending design waits
    let (ok, up) = hale_env(&["dna", "upgrade"], &app, &[("HALE_DNA_DESIGN_SUFFIX", " (a later toolchain)")]);
    assert!(ok, "{up}");
    assert!(up.contains("operating  7 practice(s) proposed (1 superseding an earlier version)"), "{up}");
    assert!(up.contains("design  8 practice(s) changed but wait"), "{up}");
    let later = review_of("operating/row-first");
    assert_ne!(later, row_first, "a new Review for the changed text");
    let new = digest_of(&later);
    assert_eq!(supersedes_of(&app, &new), old, "the replacement names the active version");
    let mut host = start_org(&app);
    let (ok, a) = hale(&["dna", "review", &later, "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && a.contains("settled: approve by riley"), "{a}");
    finish(&app, &mut host, "the replacement ratified and the old version retired", |rows| has_row(rows, "knowledge.ratified", &new) && has_row(rows, "knowledge.retired", &old));
    let (included, ctx) = package(&app);
    assert_eq!(included, vec![new], "the replacement, and not the retired version: {ctx}");
    unmigrate(&app, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

/// The receipt document a proposal's digest names, as JSON.
fn receipt(app: &Path, digest: &str) -> serde_json::Value {
    let raw = digest.strip_prefix("sha256:").unwrap_or(digest);
    let out = Command::new("git").args(["cat-file", "-p", &format!("refs/dna/receipts/{raw}")]).current_dir(app).output().unwrap();
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap_or(serde_json::Value::Null)
}

/// The proposals of the family `library`, as (digest, body).
fn library_proposals(rows: &Rows) -> Vec<(String, serde_json::Value)> {
    rows.iter()
        .filter(|r| r.0 == "knowledge.proposed")
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.2).ok().map(|b| (r.1.clone(), b)))
        .filter(|(_, b)| b["name"].as_str().unwrap_or("").starts_with("library/"))
        .collect()
}

/// seed/pull — a default `init` writes the nodes knowledge is about, the
/// edge from the application to its language, and the smallest library as
/// proposals bound to those nodes, one Review per idea under `library`;
/// `--no-library` writes none of it.
#[test]
fn init_seeds_the_language_and_system_nodes_and_the_library() {
    let _t = trace::test("dna_design::library_seed");
    let d = std::env::temp_dir().join(format!("hale_dna_library_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "libbed"], &d);
    assert!(ok, "{out}");
    assert!(out.contains("seeded  graph (language:hale, system:dna, application:libbed") && out.contains("seeded  edge (written_in") && out.contains("seeded  library (2 idea(s) proposed"), "{out}");
    let app: PathBuf = d.join("libbed");
    let rows = journal(&app);
    let body = |kind: &str, entity: &str| -> serde_json::Value {
        let r = rows.iter().find(|r| r.0 == kind && r.1 == entity).unwrap_or_else(|| panic!("no {kind} {entity}"));
        serde_json::from_str(&r.2).unwrap()
    };
    let version = env!("CARGO_PKG_VERSION");
    let language = body("graph.node", "language:hale");
    assert!(language["kind"] == "language" && language["name"] == "hale" && language["text"].as_str().unwrap().contains(&format!("Hale {version}")), "{language}");
    assert_eq!(body("graph.node", "system:dna")["kind"], "system");
    // the record has no node for an attached application: the seed adds
    // `application:<account name>` beside the edge that uses it
    assert_eq!(body("graph.node", "application:libbed")["kind"], "application");
    let edge = body("graph.edge", "written_in:application:libbed|language:hale");
    assert_eq!(edge["members"][0]["node"], "application:libbed");
    assert_eq!(edge["members"][1]["node"], "language:hale");
    assert_eq!(count_of(&rows, "graph.node"), 3, "the two nodes and the application");
    assert_eq!(count_of(&rows, "graph.edge"), 1);

    // two ideas, each bound to its node by the proposal's target, each with
    // its own Review under the family `library`
    let proposed = library_proposals(&rows);
    assert_eq!(proposed.len(), 2, "two library proposals: {proposed:?}");
    for (digest, b) in &proposed {
        let (want_text, want_target) = if b["name"] == "library/api" {
            (include_str!("../../../docs/src/services/api.md"), "language:hale")
        } else {
            (include_str!("../../../docs/src/dna/shaping.md"), "system:dna")
        };
        assert_eq!(b["target"], want_target, "{b}");
        assert!(b["kind"] == "idea" && b["class"] == "goal" && b["provenance"] == "toolchain", "{b}");
        let doc = receipt(&app, digest);
        assert_eq!(doc["text"], want_text, "the chapter's text, as the toolchain ships it");
        assert_eq!(doc["toolchain"], version, "the toolchain's version is on the idea");
        assert_eq!(doc["target"], want_target);
        let review = rows
            .iter()
            .find(|r| r.0 == "review.requested" && serde_json::from_str::<serde_json::Value>(&r.2).map(|q| q["knowledge_digest"] == digest.as_str()).unwrap_or(false))
            .expect("its Review");
        let q: serde_json::Value = serde_json::from_str(&review.2).unwrap();
        assert!(q["group"] == "library" && q["required_authority"] == "board" && q["target"] == want_target, "{q}");
    }
    // `hale dna review` lists the family, and a group verdict names it
    let (ok, list) = hale(&["dna", "review"], &app);
    assert!(ok && family_ids(&list, "library").len() == 2 && list.contains("library — 2 library idea(s)") && list.contains("hale dna review library approve|reject"), "{list}");

    // a Review of a node's idea renders the reviewer's brief for that node:
    // read from memory, so without it the line says it was not read
    let api_review = rows.iter().find(|r| r.0 == "review.requested" && r.2.contains("library/api")).map(|r| r.1.strip_prefix("review:").unwrap().to_string()).unwrap();
    let (ok, view) = hale(&["dna", "review", &api_review], &app);
    assert!(ok && view.contains("knowledge for language:hale: not read (no memory:"), "{view}");
    let design_review = rows.iter().find(|r| r.0 == "review.requested" && r.2.contains("design/principles")).map(|r| r.1.strip_prefix("review:").unwrap().to_string()).unwrap();
    let (ok, view) = hale(&["dna", "review", &design_review], &app);
    assert!(ok && !view.contains("knowledge for"), "a practice bound to a path has no node to read for: {view}");

    // --no-library: none of it, and the output says so
    let (ok, out) = hale(&["dna", "new", "bare", "--no-library"], &d);
    assert!(ok && out.contains("skipped library") && !out.contains("seeded  library"), "{out}");
    let bare = journal(&d.join("bare"));
    assert_eq!(count_of(&bare, "graph.node") + count_of(&bare, "graph.edge"), 0, "no node, no edge");
    assert!(library_proposals(&bare).is_empty(), "no library idea");
    assert!(!bare.iter().any(|r| r.0 == "review.requested" && r.2.replace(' ', "").contains("\"group\":\"library\"")), "no library Review");
    assert_eq!(count_of(&bare, "knowledge.proposed"), 1 + 8 + 7, "the purpose and the fifteen practices, as before");
    let (_, list) = hale(&["dna", "review"], &d.join("bare"));
    assert!(!list.contains("library —"), "{list}");
    let _ = std::fs::remove_dir_all(&d);
}

/// seed/pull — `hale dna review library approve` ratifies both ideas and
/// binds both: a package for the node is the idea bound to it, a package for
/// a path is neither, and `language:hale` is not `language:hale-x`.
#[test]
fn approving_the_library_ratifies_and_binds_both_ideas() {
    let _t = trace::test("dna_design::library_approve");
    let Some(owner) = owner_dsn() else {
        eprintln!("dna_design: no HALE_DNA_MEMORY_DSN_OWNER; the package is memory's, so nothing was exercised");
        return;
    };
    let Some(_nats_owner) = nats_owner_url() else {
        eprintln!("dna_design: no HALE_DNA_NATS_URL_OWNER; a verdict cannot reach the organization, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_libapprove_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "libapp"], &d);
    assert!(ok && out.contains("seeded  library (2 idea(s) proposed"), "{out}");
    let app: PathBuf = d.join("libapp");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    let proposed = library_proposals(&journal(&app));
    let digest_named = |name: &str| -> String { proposed.iter().find(|(_, b)| b["name"] == name).map(|(d, _)| d.clone()).unwrap() };
    let (api, shaping) = (digest_named("library/api"), digest_named("library/shaping"));
    let mut host = start_org(&app);
    let (ok, rest) = hale(&["dna", "review", "library", "approve", "--as", "riley", "--authority", "board"], &app);
    assert!(ok && rest.matches("settled: approve by riley").count() == 2, "both, each its own verdict:\n{rest}");
    finish(&app, &mut host, "both library ideas ratified", |rows| has_row(rows, "knowledge.ratified", &api) && has_row(rows, "knowledge.ratified", &shaping));
    assert_eq!(package_for(&app, "language:hale").0, vec![api.clone()], "the node's idea, and not the other's");
    assert_eq!(package_for(&app, "system:dna").0, vec![shaping.clone()]);
    assert_eq!(package_for(&app, "org").0, Vec::<String>::new(), "a path is neither: nothing is bound to org");
    assert_eq!(package_for(&app, "org language:hale-x").0, Vec::<String>::new(), "language:hale-x is not language:hale");
    let both = package_for(&app, "org/app/main language:hale system:dna").0;
    assert!(both.len() == 2 && both.contains(&api) && both.contains(&shaping), "a set of targets is one package: {both:?}");
    // the reviewer's brief for a Review of a node's idea names the ideas
    // already ratified for the node
    let api_review = journal(&app).iter().find(|r| r.0 == "review.requested" && r.2.contains(api.as_str())).map(|r| r.1.strip_prefix("review:").unwrap().to_string()).unwrap();
    let head = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_HEAD=");
    let mut host = start_org(&app);
    let mut brief = String::new();
    trace::wait_until("the reviewer's brief names the ratified idea", Duration::from_secs(120), Duration::from_millis(250), || {
        let (ok, out) = hale_env(&["dna", "review", &api_review], &app, &[("HALE_DNA_MEMORY_DSN_HEAD", head.as_str())]);
        brief = out;
        ok && brief.contains("- library/api")
    });
    finish(&app, &mut host, "the record", |_| true);
    assert!(brief.contains("knowledge for language:hale (1 idea(s), package "), "{brief}");
    unmigrate(&app, &owner);
    let _ = std::fs::remove_dir_all(&d);
}
