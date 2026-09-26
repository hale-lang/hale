//! GH #528 — `hale dna run` (Track C, PR 25): the stateless host.
//! It cuts a fresh artifact, builds, execs the organism under
//! LOTUS_OBS=1 from the project root, waits for the nerves (GH #986),
//! attaches plain iris (law + review-vs-baseline) to inspect the
//! organism like any Hale binary, and holds no state of its own: an
//! intent offered over the nerves lands in the organism's Journal, not
//! in the host. Iris carries nothing of DNA: no status (#998).

#[path = "support/reap.rs"]
mod reap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &std::path::Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn http(port: u16, req: &str) -> String {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.write_all(req.as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

#[test]
fn run_hosts_the_organism_with_plain_iris_and_holds_no_state() {
    let Some(_nats_owner) = std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()) else {
        eprintln!("dna_run: no HALE_DNA_NATS_URL_OWNER; an ask cannot reach the organism, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_run_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgrun"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgrun");
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (ok, migrated) = hale(&["dna", "nerves", "migrate"], &app);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--port", &port.to_string()])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_DNA_NATS_URL_SPINE", &nats_spine)
        .env("HALE_DNA_NATS_ORG", &nats_org)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hale dna run");
    // iris comes up with the current artifact and the review view
    // The observer discovers EVERY LOTUS_OBS=1 process on the machine
    // (a CI shard runs other observed tests beside this one), so wait
    // for and assert on the organism's own process by name.
    let organism_loci = |s: &str| -> Option<Vec<String>> {
        let json = s.find("{\"ts\"")?;
        let v: serde_json::Value = serde_json::from_str(&s[json..]).ok()?;
        let p = v["processes"].as_array()?.iter().find(|p| p["name"] == "org")?;
        Some(p["loci"].as_array()?.iter().map(|l| l["type"].as_str().unwrap_or("").to_string()).collect())
    };
    // the host's build when this toolchain's embedded DNA is new to the
    // cache, the organization's against the whole vendored core, then iris
    // over its process: the bound dna_knowledge gives the same start
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut snap = String::new();
    while Instant::now() < deadline {
        let s = http(port, "GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n");
        // the diff reports loaded, the law verified, and the organism's
        // own locus tree replayed (a process attaches before its births
        // are replayed, so wait for a type)
        let tree_up = organism_loci(&s).map(|t| !t.is_empty()).unwrap_or(false);
        if s.contains("\"state\":\"loaded\"") && s.contains("\"digest\":\"verified\"") && tree_up {
            snap = s;
            break;
        }
        if let Ok(Some(st)) = host.try_wait() {
            panic!("hale dna run exited early: {st}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let finish = |host: &mut std::process::Child| {
        // the host's children are the organization and iris; end the org first
        if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
        let _ = host.wait();
        let _ = Command::new("pkill").args(["-x", "fuse-hl"]).status();
    };
    if snap.is_empty() {
        finish(&mut host);
        panic!("iris did not come up with law + review over the organism's process");
    }
    assert!(app.join(".hale/dna/current.topology").is_file(), "a fresh artifact of what runs");
    // The baseline is the application as `init` found it, and init changes
    // nothing in the application (GH #566 F2): the first run's review view
    // is identical.
    let compact = snap.replace(' ', "");
    assert!(compact.contains("\"classification\":\"identical\""), "init leaves the application's model unchanged: {snap}");
    // Iris is the inspector and nothing more: the snapshot carries no
    // organism status.
    assert!(!compact.contains("\"dna\":{"), "iris carries nothing of DNA: {snap}");
    // The observed tower names imported loci by their AUTHOR-facing
    // names, so the DNA's loci join with the artifact (PR 28).
    let types = organism_loci(&snap).expect("the organism's process in the snapshot");
    assert!(types.iter().any(|t| t == "dna::Dna" || t == "dna::Metabolism"), "imported loci observed under their author-facing names: {types:?}");
    assert!(!types.iter().any(|t| t.starts_with("__lib_")), "no mangled locus type names in the observation: {types:?}");
    // an intent over the nerves (`hale dna task create`) lands in the
    // organism's Journal
    let record = |app: &Path| -> String { Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default() };
    let before = record(&app).lines().count();
    let ask = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "task", "create", "write", "the", "changelog"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    let ask_out = format!("{}{}", String::from_utf8_lossy(&ask.stdout), String::from_utf8_lossy(&ask.stderr));
    let grew = {
        let dl = Instant::now() + Duration::from_secs(15);
        loop {
            let j = record(&app);
            if j.lines().count() > before && j.contains("\"task.born\"") {
                break true;
            }
            if Instant::now() > dl {
                break false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    // the status projection followed the Journal
    let status_ok = {
        let dl = Instant::now() + Duration::from_secs(10);
        loop {
            let st = std::fs::read_to_string(app.join(".hale/dna/status.json")).unwrap_or_default();
            if st.contains("\"t1\"") {
                break true;
            }
            if Instant::now() > dl {
                break false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    finish(&mut host);
    assert!(grew, "the organism journaled intent.offered + task.born");
    assert!(ask.status.success() && ask_out.contains("task t1 born"), "task create over the nerves: {ask_out}");
    assert!(status_ok, "status.json re-projected from the Journal");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn run_refuses_a_project_without_dna() {
    let d = std::env::temp_dir().join(format!("hale_dna_run_nodna_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("main.hl"), "fn main() { }\n").unwrap();
    let (ok, out) = hale(&["dna", "run", "."], &d);
    assert!(!ok && out.contains("no DNA"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
