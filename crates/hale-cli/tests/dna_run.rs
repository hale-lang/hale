//! GH #528 — `hale dna run` (Track C, PR 25): the stateless host.
//! It cuts a fresh artifact, builds, execs the organism under
//! LOTUS_OBS=1 from the project root, waits for the membrane, attaches
//! iris (law + review-vs-baseline + membrane), and holds no state of
//! its own: an intent offered through the membrane lands in the
//! organism's Journal, not in the host.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
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
fn run_hosts_the_organism_with_iris_and_the_membrane_and_holds_no_state() {
    let d = std::env::temp_dir().join(format!("hale_dna_run_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgrun"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgrun");
    let cache = d.join("cache");
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--port", &port.to_string()])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hale dna run");
    // iris comes up with the membrane, the current artifact and the review view
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut snap = String::new();
    while Instant::now() < deadline {
        let s = http(port, "GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n");
        // the diff AND the organism's status both report loaded, the law verified
        if s.contains("\"membrane\"") && s.matches("\"state\":\"loaded\"").count() >= 2 && s.contains("\"digest\":\"verified\"") && s.contains("\"processes\":[{") {
            snap = s;
            break;
        }
        if let Ok(Some(st)) = host.try_wait() {
            panic!("hale dna run exited early: {st}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let finish = |host: &mut std::process::Child| {
        // the host's children are the organism and iris; end the organism first
        let _ = Command::new("pkill").args(["-x", "orgrun"]).status();
        let _ = host.wait();
        let _ = Command::new("pkill").args(["-x", "fuse-hl"]).status();
    };
    if snap.is_empty() {
        finish(&mut host);
        panic!("iris did not come up with membrane + law + review");
    }
    assert!(app.join(".hale/dna/current.topology").is_file(), "a fresh artifact of what runs");
    // The baseline is the application as `init` found it, BEFORE the
    // DNA was grafted on — so the first run's review view shows
    // exactly what init added: the Genome and the membrane.
    let compact = snap.replace(' ', "");
    assert!(compact.contains("\"classification\":\"model-shape\""), "{snap}");
    assert!(compact.contains("\"change\":\"added\"") && compact.contains("genome::Genome"), "the review view names the added Genome: {snap}");
    // B7: the organism's status projection rides in the snapshot
    // (state loaded, the baseline review pending) — and the observed
    // tower names imported loci by their AUTHOR-facing names, so the
    // DNA is visible and joins with the artifact (PR 28).
    let compact = snap.replace(' ', "");
    assert!(compact.contains("\"dna\":{"), "{snap}");
    assert!(compact.contains("\"reviews\":[{") && compact.contains("\"state\":\"pending\""), "the status projection carries the pending purpose review: {snap}");
    assert!(compact.contains("\"type\":\"dna::Dna\"") || compact.contains("\"type\":\"dna::Metabolism\""), "imported loci observed under their author-facing names: {snap}");
    assert!(!compact.contains("\"type\":\"__lib_"), "no mangled locus type names in the observation: {snap}");
    // an intent through the membrane lands in the organism's Journal
    let before = std::fs::read_to_string(app.join(".hale/dna/journal.jsonl")).unwrap().lines().count();
    let body = r#"{"intent_id":"i1","outcome":"write the changelog","from":"test"}"#;
    let r = http(port, &format!("POST /ctl/intent HTTP/1.0\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
    assert!(r.contains("200"), "{r}");
    // …and so does `hale dna ask` WHILE iris holds the membrane (F.13:
    // it goes through iris's /ctl, and the answer comes from the Journal)
    let ask = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "ask", "also", "tag", "the", "release"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    let ask_out = format!("{}{}", String::from_utf8_lossy(&ask.stdout), String::from_utf8_lossy(&ask.stderr));
    let grew = {
        let dl = Instant::now() + Duration::from_secs(15);
        loop {
            let j = std::fs::read_to_string(app.join(".hale/dna/journal.jsonl")).unwrap_or_default();
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
    assert!(ask.status.success() && ask_out.contains("task t2 born"), "ask through iris: {ask_out}");
    assert!(status_ok, "status.json re-projected from the Journal");
    assert!(!app.join(".hale/dna/iris.port").exists(), "the port file is gone with the host");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn run_refuses_a_project_without_dna() {
    let d = std::env::temp_dir().join(format!("hale_dna_run_nodna_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("main.hl"), "fn main() { }\n").unwrap();
    let (ok, out) = hale(&["dna", "run", "."], &d);
    assert!(!ok && out.contains("no DNA"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
