//! GH #527 B3: `hale iris` ships in the binary. The sources are
//! materialized into a toolchain-hashed cache, built with this same
//! compiler, and exec'd. These tests use a private XDG_CACHE_HOME so
//! they never touch the developer's cache.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn cache_root() -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale-iris-cli-test-{}", std::process::id()));
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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
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
    assert!(dir.join("render/web/app.js").is_file(), "web assets materialized");
    assert!(dir.join("emitter/protocol.h").is_file(), "protocol header materialized");

    // Second time is exec-only: no build banner.
    let (ok, _, err) = hale(&cache, &["iris", "--build-only"]);
    assert!(ok);
    assert!(!err.contains("building the observer"), "second launch must not rebuild: {err}");

    // Launch and fetch /snapshot.
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["iris", &port.to_string()])
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hale iris");
    let deadline = Instant::now() + Duration::from_secs(20);
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
    let _ = std::fs::remove_dir_all(&cache);
    assert!(body.contains("\"processes\""), "fuse-hl served a snapshot: {body:?}");
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
    assert!(cache.join("hale/iris").exists(), "inspector materialized under the private cache");
    let _ = (out, std::fs::remove_dir_all(&cache));
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
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["iris", &port.to_string(), "--diff"])
        .arg(&artifacts[0])
        .arg(&artifacts[1])
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hale iris --diff");
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut body = String::new();
    while Instant::now() < deadline {
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
    let _ = std::fs::remove_dir_all(&cache);
    let json_start = body.find("{\"ts\"").expect("snapshot body");
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
