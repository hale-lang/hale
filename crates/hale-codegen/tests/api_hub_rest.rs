//! GH #1780: a `ws::Hub` mounted on an `http::Rest` exposure's listener
//! (`hubs: [("/hub", self.hub)]`) answers on the Rest's one bound address.
//! Plain HTTP is the surface's; a WebSocket upgrade at the mounted path is
//! handed to the hub, whose sources, roles and `requires:` judge it as on a
//! listener of its own, and which opens no listener of its own (the
//! harness's `Server::start` finds exactly one on the loopback). The Rest
//! exposure's `GET /.description` lists the hub's streams for a caller who
//! may subscribe to them, and a connection the Rest listener handed over
//! is not ended by the 5 s read clock of a Rest request.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ws_hub.rs"]
mod ws_hub;
#[path = "support/ws_client.rs"]
mod ws_client;

use ws_client::*;
use ws_hub::*;

const PROGRAM: &str = r#"
role operator;

type Fill { qty: Int; price: Int; }
topic Fills { payload: Fill; subject: "desk.fills"; }
type Health { status: String; }

api Desk {
    rpc Probe::live at "GET /healthz";
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-dave" { return std::api::Principal { mode: "bearer", name: "dave" }; }
        if token == "t-bob" { return std::api::Principal { mode: "bearer", name: "bob" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Grants {
    params { operator: String = ""; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return len(self.operator) > 0 && p.name == self.operator; }
        return false;
    }
}

locus Probe {
    fn live() -> Health { return Health { status: "live" }; }
}

locus Maker {
    params { made: Int = 0; }
    bus { publish Fills; }
    fn make(n: Int) {
        self.made = self.made + 1;
        Fills <- Fill { qty: n, price: n * 10 };
    }
}

main locus App {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave" };
        hub: ws::Hub = ws::Hub { principals: self.bearer, roles: self.hub_roles, as: "fills" };
        maker: Maker = Maker { };
        probe: Probe = Probe { };
        web: api::Exposure = api::serve(Desk, http::Rest { bind: std::env::var("BIND"), codec: json, principals: self.bearer, roles: self.hub_roles, anonymous: "anon", hubs: [("/hub", self.hub)] }, as: "web", bound: 16, on_full: refuse);
    }
    bindings {
        Fills: self.hub requires: [operator], bound: 8, on_full: drop_old;
    }
    placement {
        web: cooperative(pool = edge) where async_io;
    }
    run() {
        let ctl = std::env::var("CTL");
        let mut done = false;
        while !done && !self.draining {
            let cmd = std::str::trim(std::io::fs::read_file(ctl + "/cmd") or "");
            if len(cmd) > 0 {
                std::io::fs::unlink(ctl + "/cmd") or discard;
                if cmd == "stop" { done = true; }
                if std::str::starts_with(cmd, "publish ") {
                    let n = std::str::parse_int(cmd[8..len(cmd)]) or 0;
                    let mut i = 1;
                    while i <= n {
                        self.maker.make(i);
                        i = i + 1;
                    }
                }
                std::io::fs::write_file(ctl + "/stats", "made=" + to_string(self.maker.made) + "\nevents=" + to_string(self.hub.events()) + "\nsubscribers=" + to_string(self.hub.subscribers()) + "\n") or discard;
            }
            std::time::sleep(5ms);
        }
        self.web.stop();
        self.hub.stop();
    }
}

fn main() { App { }; }
"#;

fn build() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = harness::unique_bin("api_hub_rest");
        build_opts::build_source(PROGRAM, &bin, &build_opts::options()).expect("build the program");
        bin
    })
    .clone()
}

/// A plain HTTP request on the Rest port: the status and the body.
fn http(port: u16, method: &str, path: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).expect("timeout");
    s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut all = Vec::new();
    s.read_to_end(&mut all).expect("read");
    let text = String::from_utf8_lossy(&all).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0), body.to_string())
}

/// A WebSocket at the mounted path, upgraded.
fn hub(port: u16, token: Option<&str>) -> Ws {
    let extra = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let (ws, status) = Ws::try_connect(port, &extra, "/hub");
    assert!(status.starts_with("HTTP/1.1 101"), "the upgrade at the mount was refused: {}", ws.head);
    assert_eq!(ws.accept_key(), RFC_ACCEPT);
    ws
}

#[test]
fn a_hub_mounted_on_a_rest_listener_streams_on_its_port() {
    let server = Server::start(&build(), &[]);
    let port = server.port;
    // plain HTTP is the surface's
    assert_eq!(http(port, "GET", "/healthz"), (200, "{\"status\":\"live\"}".to_string()));
    // a WebSocket upgrade at the mount is the hub's
    let mut ws = hub(port, Some("t-dave"));
    ws.subscribe("Fills");
    assert_eq!(ws.text(), "{\"type\":\"subscribed\",\"topic\":\"Fills\"}");
    // the surface still answers while the hub's connection is open
    assert_eq!(http(port, "GET", "/healthz").0, 200);
    // quiet for longer than a Rest request may take to arrive: the hub's
    // connection is not on that clock
    std::thread::sleep(Duration::from_millis(6000));
    server.command("publish 2");
    server.await_stat("events", 2);
    assert_eq!(ws.text(), "{\"type\":\"event\",\"topic\":\"Fills\",\"seq\":1,\"payload\":{\"qty\":1,\"price\":10}}");
    assert_eq!(ws.text(), "{\"type\":\"event\",\"topic\":\"Fills\",\"seq\":2,\"payload\":{\"qty\":2,\"price\":20}}");
    // the mount's path without an upgrade is the surface's: no row serves it
    assert_eq!(http(port, "GET", "/hub").0, 404);
    // the hub's stop closes the connection it took over, with `closed`
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert_eq!(ws.text(), "{\"type\":\"closed\",\"reason\":\"shutting_down\"}");
    assert!(matches!(ws.recv_within(5000), Frame::Close(_)));
}

#[test]
fn a_mounted_hub_judges_its_callers_as_on_its_own_listener() {
    let server = Server::start(&build(), &[]);
    let port = server.port;
    // a caller who does not hold `operator`
    let mut bob = hub(port, Some("t-bob"));
    bob.subscribe("Fills");
    assert_eq!(
        bob.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Fills requires operator\",\"requires\":[\"operator\"]}}"
    );
    // a token the bearer source refuses, and none
    let mut bad = hub(port, Some("t-ghost"));
    bad.subscribe("Fills");
    assert_eq!(
        bad.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthenticated\",\"reason\":\"the bearer token is refused: no such token\"}}"
    );
    let mut none = hub(port, None);
    none.subscribe("Fills");
    assert!(none.text().contains("\"kind\":\"unauthenticated\""));
    // nobody was admitted: a publish reaches no one
    server.command("publish 1");
    server.await_stat("subscribers", 0);
    bob.silence(300);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn the_rest_description_lists_the_mounted_hubs_streams() {
    let server = Server::start(&build(), &[]);
    let port = server.port;
    let (status, dave) = get_description(port, Some("t-dave"));
    assert_eq!(status, 200, "{dave}");
    assert!(dave.contains("\"name\":\"web\""), "{dave}");
    assert!(dave.contains("\"roles\":[\"operator\"]"), "{dave}");
    assert!(
        dave.contains("\"streams\":[{\"topic\":\"Fills\",\"subject\":\"desk.fills\",\"direction\":\"out\",\"payload\":{\"$ref\":\"#/schemas/Fill\"}"),
        "{dave}"
    );
    assert!(dave.contains("\"requires\":[\"operator\"]}]"), "{dave}");
    assert!(dave.contains("\"Fill\":{\"type\":\"object\""), "the stream's schema: {dave}");
    assert!(dave.contains("\"Health\":{"), "the row's schema: {dave}");
    // a caller who may not subscribe is not shown the stream, nor its schema
    let (status, bob) = get_description(port, Some("t-bob"));
    assert_eq!(status, 200, "{bob}");
    assert!(bob.contains("\"streams\":[]"), "{bob}");
    assert!(!bob.contains("\"Fill\":"), "{bob}");
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}
