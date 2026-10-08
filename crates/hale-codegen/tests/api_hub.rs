//! GH #1417 (R5): a topic bound to a hub is a stream.
//!
//! `HUB` is a program whose main locus holds a `ws::Hub` and binds the topic
//! `Fills` to it (`Fills: self.hub requires: [operator], bound: 8, on_full:
//! drop_old;`). It is driven by files (`support/ws_hub.rs`): it publishes
//! when told to, writes what a test may wait for, and ends when told to.
//! These tests hold the binding to its row: a build accepts it, the hub's
//! listener is bound in the program's birth (and a program that cannot bind
//! it does not boot), and a publish on the topic reaches the hub as it
//! reaches any adapter, once per publish and in order.

use std::path::PathBuf;
use std::sync::OnceLock;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/ws_hub.rs"]
mod ws_hub;

use ws_hub::*;

pub const HUB: &str = r#"
role operator;

type Fill { qty: Int; price: Int; }
topic Fills { payload: Fill; subject: "desk.fills"; }

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-dave" { return std::api::Principal { mode: "bearer", name: "dave" }; }
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

locus Maker {
    params { made: Int = 0; }
    bus { publish Fills; }
    fn make(n: Int) {
        self.made = self.made + 1;
        Fills <- Fill { qty: n, price: n * 10 };
    }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave" };
        hub: ws::Hub = ws::Hub { bind: std::env::var("BIND"), principals: self.bearer, roles: self.hub_roles, as: "fills" };
        maker: Maker = Maker { };
    }
    bindings {
        Fills: self.hub requires: [operator], bound: 8, on_full: drop_old;
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
                std::io::fs::write_file(ctl + "/stats", "made=" + to_string(self.maker.made) + "\nevents=" + to_string(self.hub.events()) + "\n") or discard;
            }
            std::time::sleep(5ms);
        }
        println("events=", self.hub.events(), " made=", self.maker.made);
        self.hub.stop();
    }
}

fn main() { Desk { }; }
"#;

fn build() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = harness::unique_bin("api_hub");
        build_opts::build_source(HUB, &bin, &build_opts::options()).expect("build the hub program");
        bin
    })
    .clone()
}

#[test]
fn a_publish_on_a_topic_bound_to_a_hub_reaches_the_hub_once_each() {
    let server = Server::start(&build(), &[]);
    server.command("publish 5");
    server.await_stat("events", 5);
    server.command("publish 3");
    server.await_stat("events", 8);
    assert_eq!(server.stat("made"), Some(8));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert!(done.stdout.contains("events=8 made=8"), "{}", done.stdout);
}

#[test]
fn a_hub_that_cannot_bind_its_address_fails_the_boot() {
    let first = Server::start(&build(), &[]);
    // a second program for the same address: the listener's birth refuses it
    let dir = harness::unique_dir("wshub_second");
    let mut second = Server::start_on(&build(), &[], first.port, dir);
    let done = second.wait(std::time::Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "{}{}", done.stdout, done.stderr);
    assert!(
        done.stderr.contains(&format!("api: ws::Hub could not listen on 127.0.0.1:{}", first.port)),
        "{}",
        done.stderr
    );
    // the first keeps its address
    assert!(std::net::TcpStream::connect(("127.0.0.1", first.port)).is_ok());
    let done = first.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn a_stopped_hub_releases_its_address() {
    let server = Server::start(&build(), &[]);
    let port = server.port;
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    // nothing listens any more
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
}
