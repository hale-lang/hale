//! The TCP connection substrate as the checker sees it (spec/stdlib.md §
//! "The TCP connection substrate"): a program that places a listener and
//! its consumers apart checks clean, it may state that no accepted
//! connection is dropped in silence (`require no_silent_loss` over the
//! substrate's topic holds, since a handle no consumer takes reaches its
//! listener, which closes it), and it stops a listener on another pool with
//! `std::io::tcp::stop(name)` where a method call across pools is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

fn check(src: &str, tag: &str, flags: &[&str]) -> (bool, String, PathBuf) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_tcp_substrate_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), src).unwrap();
    let target = d.join("main.hl").to_string_lossy().to_string();
    let mut args: Vec<String> = vec!["check".into()];
    args.extend(flags.iter().map(|f| f.replace("{dir}", &d.to_string_lossy())));
    args.push(target);
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(&args).current_dir(Path::new("/")).output().expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text, d)
}

const REPLICAS: &str = r#"
locus Api {
    params { tag: String = ""; }
    fn handle(req: std::http::Request) -> std::http::Response {
        return std::http::Response { status: 200, body: self.tag };
    }
}
main locus App {
    params {
        zero: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 0, handler: std::http::Responder { handler: Api { tag: "zero" } } };
        one: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 1, handler: std::http::Responder { handler: Api { tag: "one" } } };
        acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor { port: 8080, name: "web", shards: 2 };
    }
    placement {
        zero: cooperative(pool = zero) where async_io;
        one: cooperative(pool = one) where async_io;
        acceptor: pinned;
    }
    claims {
        handed_over: require no_silent_loss(topic __TcpAcceptedT);
    }
    run() {
        while !self.draining { std::time::sleep(20ms); }
        std::io::tcp::stop("web");
    }
}
fn main() { App { }; }
"#;

#[test]
fn a_listener_and_its_replicas_check_clean_and_lose_no_connection_in_silence() {
    let (ok, out, d) = check(REPLICAS, "replicas", &["--dump-topology={dir}/t.json"]);
    assert!(ok, "{out}");
    assert!(!out.contains("warning"), "no finding: {out}");
    let artifact = std::fs::read_to_string(d.join("t.json")).expect("the artifact");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value = serde_json::from_str(&artifact).expect("json");
    let claim = v["claims"].as_array().unwrap().iter().find(|c| c["name"] == "handed_over").expect("the claim");
    assert_eq!(claim["result"], "holds", "{claim}");
}

const SERVER_STOP: &str = r#"
locus Api {
    fn handle(req: std::http::Request) -> std::http::Response {
        return std::http::Response { status: 200, body: "ok" };
    }
}
main locus App {
    params {
        web: std::http::Server = std::http::Server { port: 8080, name: "web", handler: Api { } };
    }
    placement { web: cooperative(pool = web) where async_io; }
    run() {
        while !self.draining { std::time::sleep(20ms); }
        std::io::tcp::stop("web");
    }
}
fn main() { App { }; }
"#;

#[test]
fn a_server_on_another_pool_is_stopped_by_name() {
    // The program as written stops the server by name; the harvested
    // corpus reads it so, and the second case swaps the one statement.
    let (ok, out, d) = check(SERVER_STOP, "stop", &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert!(ok && !out.contains("warning"), "stop(name) is a free call, pool-neutral: {out}");
    let (ok, out, d) = check(&SERVER_STOP.replace("std::io::tcp::stop(\"web\")", "self.web.shutdown()"), "shutdown", &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert!(!ok && out.contains("cross-pool method call"), "the method across pools is refused, which is why stop exists: {out}");
}
