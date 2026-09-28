//! GH #527 B3: `hale iris` ships in the binary. The sources are
//! materialized into a toolchain-hashed cache, built with this same
//! compiler, and exec'd. These tests use a private XDG_CACHE_HOME so
//! they never touch the developer's cache.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// ONE cache for every test that launches the observer, in this
/// binary and its siblings (dna_run, dna_status): the
/// observer builds once per machine and `hale iris` serializes the
/// build with a lock, so five tests on a loaded CI shard do not each
/// compile it. Never deleted: it is toolchain-hashed, and the next
/// run wants it.
fn cache_root() -> PathBuf {
    let d = std::env::temp_dir().join("hale-tests-iris-cache");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hale(cache: &PathBuf, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .env("XDG_CACHE_HOME", cache)
        .output()
        .expect("invoke hale");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// GH #578: `fuse-hl` exited 1 "silently" on loaded CI shards — it was
/// killed by a signal, and `hale iris` turned that into status 1 without
/// a word. `hale iris` now names the signal; a log that does is a failure
/// here even when a retried attempt went on to pass, the way the startup
/// spec fails a drain that ran out its grace.
fn assert_no_signal_death(log: &str) {
    if let Some(line) = log.lines().find(|l| l.contains("was killed by signal")) {
        panic!("fuse-hl died (GH #578): {line}\n{log}");
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Start `hale iris <port> <args…>` on a free port and wait until OUR
/// server says it is listening. A port picked with bind(0) and released
/// can be taken before fuse-hl binds it — under a loaded shard, by
/// another test's server — and then a connect to the port succeeds
/// against a stranger whose snapshot never loads our diff, which a poll
/// loop waits 300 s for. So: stderr captured, the ready line
/// (`fuse-hl: listening`) awaited from our own child, an early exit or
/// a silent child retried on a fresh port, and the failure named.
fn spawn_iris(cache: &Path, args: &[&str]) -> (std::process::Child, u16) {
    let mut last = String::new();
    for _ in 0..4 {
        let port = free_port();
        let log = cache.join(format!("iris-{port}.stderr"));
        // both streams into one log: the ready line is printed, the
        // build banner and a bind failure are eprinted
        let out = std::fs::File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        // a private registry: this iris discovers no other test's observed
        // processes, whose segments come and go under it on a loaded
        // shard — what it serves here needs none of them
        let runtime = cache.join(format!("iris-{port}-runtime"));
        let _ = std::fs::create_dir_all(&runtime);
        let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(["iris", &port.to_string()])
            .args(args)
            .env("XDG_CACHE_HOME", cache)
            .env("XDG_RUNTIME_DIR", &runtime)
            .stdout(out)
            .stderr(err)
            .spawn()
            .expect("spawn hale iris");
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            if let Ok(Some(st)) = child.try_wait() {
                let said = std::fs::read_to_string(&log).unwrap_or_default();
                // a death is the failure, not a reason to retry (GH #578)
                assert_no_signal_death(&said);
                last = format!("hale iris exited {st} before serving on {port}:\n{said}");
                break;
            }
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            if said.contains("fuse-hl: listening") && TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return (child, port);
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                last = format!("hale iris did not accept on {port} within 240s:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("{last}; retrying on another port");
    }
    panic!("{last}");
}

#[test]
fn iris_materializes_builds_once_and_serves_a_snapshot() {
    let cache = cache_root();

    // --where names a directory under our private cache root.
    let (ok, out, err) = hale(&cache, &["iris", "--where"]);
    assert!(ok, "--where: {err}");
    let dir = PathBuf::from(out.trim());
    assert!(dir.starts_with(&cache), "cache dir under XDG_CACHE_HOME: {}", dir.display());

    // --build-only materializes and builds; the binary exists.
    let (ok, out, err) = hale(&cache, &["iris", "--build-only"]);
    assert!(ok, "--build-only: {err}");
    let bin = PathBuf::from(out.trim());
    assert!(bin.is_file(), "built binary at {}", bin.display());
    assert!(dir.join("iris/render/web/app.js").is_file(), "web assets materialized");
    assert!(dir.join("iris/emitter/protocol.h").is_file(), "protocol header materialized");

    // Second time is exec-only: no build banner.
    let (ok, _, err) = hale(&cache, &["iris", "--build-only"]);
    assert!(ok);
    assert!(!err.contains("building the observer"), "second launch must not rebuild: {err}");

    // Launch and fetch /snapshot.
    let (mut child, port) = spawn_iris(&cache, &[]);
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut body = String::new();
    while Instant::now() < deadline {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
            let _ = s.write_all(b"GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n");
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            if buf.contains("200") && buf.contains("processes") {
                body = buf;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(body.contains("\"processes\""), "fuse-hl served a snapshot: {body:?}");
}

/// The DNA toolchain cache is restored from a GitHub Actions cache
/// entry before a warm step runs; a truncated or zero-byte binary
/// from an incomplete save must not be trusted as already built —
/// `ensure_built_in`'s existence check now verifies the file is
/// non-empty, not just present, and rebuilds if not.
#[test]
fn a_truncated_cached_binary_is_rebuilt_not_trusted() {
    let cache = std::env::temp_dir().join(format!("hale-tests-iris-truncated-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).unwrap();

    let (ok, out, err) = hale(&cache, &["iris", "--build-only"]);
    assert!(ok, "--build-only: {err}");
    let bin = PathBuf::from(out.trim());
    let real_len = std::fs::metadata(&bin).unwrap().len();
    assert!(real_len > 0, "a real build produced a non-empty binary");

    // Simulate a partial cache restore: the file is present, but empty.
    std::fs::write(&bin, []).unwrap();
    assert_eq!(std::fs::metadata(&bin).unwrap().len(), 0);

    let (ok, out2, err) = hale(&cache, &["iris", "--build-only"]);
    assert!(ok, "--build-only after truncation: {err}");
    assert!(err.contains("building the observer"), "the empty stub must be rebuilt, not trusted: {err}");
    let bin2 = PathBuf::from(out2.trim());
    let rebuilt_len = std::fs::metadata(&bin2).unwrap().len();
    assert_eq!(rebuilt_len, real_len, "the rebuild produces the same binary");

    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn iris_inspect_builds_and_reports_a_missing_artifact() {
    let cache = cache_root();
    let (ok, out, err) = hale(&cache, &["iris", "inspect", "/nonexistent/artifact.json"]);
    // The inspector exits non-zero on an unreadable artifact; what
    // matters here is that it built and RAN (its own message, not a
    // hale iris build error).
    assert!(!ok, "inspect on a missing artifact must not succeed");
    assert!(!err.contains("failed"), "inspector must have built and run: {err}");
    assert!(cache.join("hale/iris").exists(), "inspector materialized under the shared cache");
    let _ = out;
}

/// GH #527 B5: `hale iris --diff a b` diffs the pair with the
/// compiler's engine and fuse-hl carries the document into
/// /snapshot verbatim, under `diff`.
#[test]
fn iris_diff_pair_rides_into_the_snapshot() {
    let cache = cache_root();
    let src_a = "type T { n: Int = 0; }\ntopic Evt { payload: T; subject: \"evt\"; }\nlocus Worker {\n    bus { subscribe Evt as on_e; }\n    fn on_e(t: T) { println(\"e\"); }\n}\nmain locus App {\n    params { w: Worker = Worker { }; }\n    bus { publish Evt; }\n    run() { Evt <- T { n: 1 }; }\n}\nfn main() { App { }; }\n";
    let src_b = src_a.replace("locus Worker", "locus Late {\n    bus { subscribe Evt as on_l; }\n    fn on_l(t: T) { println(\"l\"); }\n}\nlocus Worker")
        .replace("params { w: Worker = Worker { }; }", "params { w: Worker = Worker { }; l: Late = Late { }; }");
    let mut artifacts = Vec::new();
    for (name, src) in [("a", src_a.to_string()), ("b", src_b)] {
        let seed = cache.join(name);
        std::fs::create_dir_all(&seed).unwrap();
        std::fs::write(seed.join("main.hl"), src).unwrap();
        let art = cache.join(format!("{name}.topology"));
        let out = Command::new(env!("CARGO_BIN_EXE_hale"))
            .arg("check")
            .arg(&seed)
            .arg(format!("--dump-topology={}", art.display()))
            .output()
            .unwrap();
        assert!(art.is_file(), "artifact {name}: {}", String::from_utf8_lossy(&out.stderr));
        artifacts.push(art);
    }
    // On a loaded CI shard the server has exited 1 within seconds of
    // `fuse-hl: listening`, printing nothing (GH #578); a workstation
    // never sees it. A server that dies before the diff loads is
    // retried on a fresh port, with what it printed kept; a server that
    // lives and never loads the diff is the failure this test is for.
    let mut body = String::new();
    let mut attempts: Vec<String> = Vec::new();
    for attempt in 0..3 {
        let (mut child, port) = spawn_iris(&cache, &["--diff", &artifacts[0].to_string_lossy(), &artifacts[1].to_string_lossy()]);
        let deadline = Instant::now() + Duration::from_secs(300);
        let mut exited: Option<String> = None;
        while Instant::now() < deadline {
            if let Ok(Some(st)) = child.try_wait() {
                exited = Some(format!("{st}"));
                break;
            }
            if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
                let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = s.write_all(b"GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n");
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                // the diff loads on the 1 Hz discovery tick
                if buf.contains("\"state\":\"loaded\"") {
                    body = buf;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = child.kill();
        let _ = child.wait();
        let log = std::fs::read_to_string(cache.join(format!("iris-{port}.stderr"))).unwrap_or_default();
        if !body.is_empty() {
            break;
        }
        attempts.push(format!("attempt {attempt} on {port}: server {}\n{log}", exited.as_deref().unwrap_or("still running, never loaded the diff")));
        if exited.is_none() {
            break;
        }
    }
    for a in &attempts {
        assert_no_signal_death(a);
    }
    if !attempts.is_empty() {
        // a retried attempt is evidence for GH #578 even when a later one passed
        eprintln!("iris_diff_pair: retried after a server died before the diff loaded:\n{}", attempts.join("\n"));
    }
    let json_start = body.find("{\"ts\"").unwrap_or_else(|| panic!("no loaded diff in the snapshot:\n{}", attempts.join("\n")));
    let v: serde_json::Value = serde_json::from_str(&body[json_start..]).expect("snapshot is JSON");
    assert_eq!(v["diff"]["state"], "loaded", "{body}");
    let doc = &v["diff"]["document"];
    assert_eq!(doc["schema"], "1.0");
    assert_eq!(doc["classification"], "model-shape");
    let added: Vec<&str> = doc["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["change"] == "added" && r["kind"] == "locus")
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(added, vec!["Late"]);
    // the B side became the law artifact
    assert_eq!(v["law"]["digest"], "verified", "{}", v["law"]);
}

/// An observed program that births loci it keeps, far past the
/// live-locus table's bound: a million in about twenty seconds, after a
/// pause that lets iris attach before its first birth.
const CHURN: &str = "locus Kid { params { n: Int = 0; } }\nlocus Kids {\n    params { }\n    accept(c: Kid) { }\n    fn add(n: Int) { Kid { n: n }; }\n}\nmain locus App {\n    params { kids: Kids = Kids { }; }\n    run() {\n        // time for iris's 1 Hz discovery to attach first\n        std::time::sleep(3s);\n        let mut i = 0;\n        while i < 1000000 && !self.draining {\n            self.kids.add(i);\n            i = i + 1;\n            if i % 100 == 0 { std::time::sleep(1ms); }\n        }\n        while !self.draining { std::time::sleep(100ms); }\n    }\n}\nfn main() { App { }; }\n";

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// The `fuse-hl` `hale iris` (pid `parent`) runs.
fn fuse_hl_of(parent: u32) -> Option<u32> {
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        // pid (comm) state ppid …
        let Some(close) = stat.rfind(')') else { continue };
        let comm = stat.get(stat.find('(').unwrap_or(0) + 1..close).unwrap_or("");
        let ppid: u32 = stat[close + 1..].split_whitespace().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        if ppid == parent && comm == "fuse-hl" {
            return Some(pid);
        }
    }
    None
}

/// A child this test started, stopped however the test ends: a failed
/// assertion must not leave `hale iris` (and its fuse-hl) or the churn
/// program running. SIGTERM first, so a Hale program drains and removes
/// its own observer segment; SIGKILL if it has not gone within 5 s.
struct Stopped(std::process::Child);
impl Drop for Stopped {
    fn drop(&mut self) {
        let pid = self.0.id();
        if let Ok(None) = self.0.try_wait() {
            let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.0.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn snapshot(port: u16) -> Option<serde_json::Value> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    s.write_all(b"GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n").ok()?;
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    serde_json::from_str(&buf[buf.find("{\"ts\"")?..]).ok()
}

/// GH #578 / the #1192 Deferred line: fuse-hl on a live organism grew to
/// 10 GB. Every event scanned the live-locus table (copying each row),
/// the table had no bound, the snapshot grew past each replaced block,
/// and every `/snapshot` answer stayed in the HTTP server's region.
/// Under a program churning a million loci, the table holds at its bound
/// and the process stays flat across requests.
#[test]
#[cfg(target_os = "linux")]
fn fuse_hl_stays_flat_under_locus_churn_and_requests() {
    let cache = cache_root();
    let seed = cache.join("churn");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), CHURN).unwrap();
    let (ok, _, err) = hale(&cache, &["build", &seed.join("main.hl").to_string_lossy()]);
    assert!(ok, "the churn program builds: {err}");
    let (iris, port) = spawn_iris(&cache, &[]);
    let iris = Stopped(iris);
    let runtime = cache.join(format!("iris-{port}-runtime"));
    let churn = Command::new(seed.join("main"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the churn program");
    let churn = Stopped(churn);
    let fuse = fuse_hl_of(iris.0.id()).expect("hale iris runs fuse-hl");
    // the table fills, then takes no more
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut full = false;
    while Instant::now() < deadline && !full {
        full = snapshot(port).map(|v| v["loci_untracked"].as_u64().unwrap_or(0) > 0).unwrap_or(false);
        std::thread::sleep(Duration::from_millis(250));
    }
    let warm = rss_kb(fuse);
    let mut rows = 0;
    let mut types: Vec<String> = Vec::new();
    for _ in 0..200 {
        if let Some(v) = snapshot(port) {
            let loci: Vec<&serde_json::Value> = v["processes"].as_array().into_iter().flatten().flat_map(|p| p["loci"].as_array().into_iter().flatten()).collect();
            rows = loci.len();
            types = loci.iter().filter_map(|l| l["type"].as_str().map(str::to_string)).collect();
            assert!(rows <= 8192, "the live-locus table holds at its bound: {rows} rows");
        }
    }
    let after = rss_kb(fuse);
    drop(churn);
    drop(iris);
    let log = std::fs::read_to_string(cache.join(format!("iris-{port}.stderr"))).unwrap_or_default();
    assert_no_signal_death(&log);
    assert!(full, "the churn never filled the table:\n{log}");
    assert_eq!(rows, 8192, "a full table shows its bound");
    // the rows kept are the ones born first: the program's long-lived
    // loci stay tracked, so their traffic stays attributed
    assert!(types.iter().any(|t| t == "App") && types.iter().any(|t| t == "Kids"), "the long-lived loci are kept: {:?}", &types[..types.len().min(8)]);
    assert!(
        after <= warm + 32 * 1024,
        "fuse-hl grew {} KB over 200 snapshots under churn ({warm} KB to {after} KB)\n{log}",
        after.saturating_sub(warm)
    );
}

/// GH #578: every way `fuse-hl` ends says why. A child killed by a signal
/// has no exit code; `hale iris` used to turn that into status 1 without
/// a word. It now names the signal and exits as a shell would.
#[test]
#[cfg(target_os = "linux")]
fn a_killed_observer_names_the_signal() {
    let cache = cache_root();
    let (mut iris, port) = spawn_iris(&cache, &[]);
    let fuse = fuse_hl_of(iris.id()).expect("hale iris runs fuse-hl");
    // this test's own fuse-hl, by its pid
    let _ = Command::new("kill").args(["-KILL", &fuse.to_string()]).status();
    let status = iris.wait().expect("hale iris exits");
    let log = std::fs::read_to_string(cache.join(format!("iris-{port}.stderr"))).unwrap_or_default();
    assert_eq!(status.code(), Some(128 + 9), "exits as a shell would: {log}");
    assert!(log.contains("hale iris: fuse-hl was killed by signal 9 (SIGKILL"), "and says so: {log}");
}
