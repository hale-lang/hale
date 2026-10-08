//! GH #1417 (R5): the stream half of the witness, over real sockets.
//!
//! `WITNESS` is the witness's hub half (`tests/api-contract/program.hl`): the
//! types `Fill`, `Money` and `OrderId`, the topic `Fills`, a bearer source
//! (`Tokens`: dave, bob, erin, and `t-soon`, whose credential expires), a role
//! source (`Grants`: dave and erin hold `operator`, and a revision is
//! announced when a grant is revoked) and a `ws::Hub` named `fills` that binds
//! `Fills` (`requires: [operator], bound: 64, on_full: drop_old`). The program
//! is driven by files (`support/ws_hub.rs`): it publishes, revokes and stops
//! when told to, and writes what a test may wait for.
//!
//! The rows are spec/api.md § Streams: a subscription is admitted against
//! the row's `requires` and answered `subscribed` or `refusal`; every event
//! offered takes the next `seq`; a subscription ends, with one
//! `unauthorized` frame and nothing delivered after, at the credential's
//! expiry and at a revision its `requires` no longer holds under;
//! `stop()` sends `closed`; and a connection that ends without `closed` is
//! the transport failure.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/ws_hub.rs"]
mod ws_hub;
#[path = "support/ws_client.rs"]
mod ws_client;

use ws_client::*;
use ws_hub::*;

pub const WITNESS: &str = r#"
role operator;
role trader;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;
type Fill { order: OrderId; qty: Int; price: Money; }
topic Fills { payload: Fill; subject: "desk.fills"; }

// Who a bearer token is. `t-soon` is erin, whose credential expires 700 ms
// after the source is first asked about it.
locus Tokens {
    params { issued: Int = 0; }
    fn principal(token: String) -> std::api::Principal {
        if token == "t-dave" { return std::api::Principal { mode: "bearer", name: "dave" }; }
        if token == "t-bob" { return std::api::Principal { mode: "bearer", name: "bob" }; }
        if token == "t-erin" || token == "t-soon" { return std::api::Principal { mode: "bearer", name: "erin" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
    fn expiry(token: String) -> Int {
        if token == "t-soon" {
            if self.issued == 0 { self.issued = std::time::nanos(std::time::current()); }
            return self.issued + 700000000;
        }
        return 0;
    }
}

// Two operators; a revoked grant announces the new revision.
locus Grants {
    params {
        operator: String = "";
        also: String = "";
        revision: Int = 1;
    }
    bus { publish "__api.roles.revision" of type std::api::Revision; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return (len(self.operator) > 0 && p.name == self.operator) || (len(self.also) > 0 && p.name == self.also); }
        return false;
    }
    fn grants(p: std::api::Principal) -> std::api::Grants {
        let mut roles = "";
        if self.holds(p, "operator") { roles = "operator"; }
        return std::api::Grants { roles: roles, revision: self.revision };
    }
    fn revoke(who: String) {
        if self.operator == who { self.operator = ""; }
        if self.also == who { self.also = ""; }
        self.revision = self.revision + 1;
        "__api.roles.revision" <- std::api::Revision { source: "hub_roles", revision: self.revision };
    }
}

locus Maker {
    params { made: Int = 0; }
    bus { publish Fills; }
    fn make(n: Int) {
        self.made = self.made + 1;
        Fills <- Fill { order: OrderId(n), qty: n, price: n * 10cent };
    }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave", also: "erin" };
        hub: ws::Hub = ws::Hub { bind: std::env::var("BIND"), principals: self.bearer, roles: self.hub_roles, as: "fills" };
        maker: Maker = Maker { };
    }
    bindings {
        Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
    }
    fn stats(ctl: String) {
        std::io::fs::write_file(ctl + "/stats.tmp", "made=" + to_string(self.maker.made) + "\nevents=" + to_string(self.hub.events()) + "\nsubs=" + to_string(self.hub.subscribers()) + "\nconns=" + to_string(self.hub.connections()) + "\nrevision=" + to_string(self.hub_roles.revision) + "\n") or discard;
        std::io::fs::rename(ctl + "/stats.tmp", ctl + "/stats") or discard;
    }
    // one command is `;`-separated steps: `fill N`, `revoke WHO`, `announce`, `stop`
    fn step(s: String) -> Bool {
        let c = std::str::trim(s);
        if c == "stop" { return true; }
        if std::str::starts_with(c, "fill ") {
            let n = std::str::parse_int(c[5..len(c)]) or 0;
            let mut i = 1;
            while i <= n {
                self.maker.make(i);
                i = i + 1;
            }
        }
        if std::str::starts_with(c, "revoke ") {
            self.hub_roles.revoke(c[7..len(c)]);
        }
        if c == "announce" {
            self.hub.announce(self.hub_roles.revision);
        }
        return false;
    }
    run() {
        let ctl = std::env::var("CTL");
        let mut done = false;
        while !done && !self.draining {
            let cmd = std::str::trim(std::io::fs::read_file(ctl + "/cmd") or "");
            if len(cmd) > 0 {
                std::io::fs::unlink(ctl + "/cmd") or discard;
                let mut rest = cmd;
                while len(rest) > 0 && !done {
                    let semi = std::str::index_of(rest, ";");
                    let mut one = rest;
                    if semi >= 0 {
                        one = rest[0..semi];
                        rest = rest[(semi + 1)..len(rest)];
                    } else {
                        rest = "";
                    }
                    if self.step(one) { done = true; }
                }
            }
            self.stats(ctl);
            std::time::sleep(5ms);
        }
        println("events=", self.hub.events(), " made=", self.maker.made);
        if std::env::var("MODE") != "scope_exit" {
            self.hub.stop();
        }
    }
}

fn main() { Desk { }; }
"#;

fn build() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = harness::unique_bin("api_hub_streams");
        build_opts::build_source(WITNESS, &bin, &build_opts::options()).expect("build the witness's hub half");
        bin
    })
    .clone()
}

fn start() -> Server {
    Server::start(&build(), &[])
}

fn event(seq: i64, n: i64) -> String {
    format!("{{\"type\":\"event\",\"topic\":\"Fills\",\"seq\":{seq},\"payload\":{{\"order\":{n},\"qty\":{n},\"price\":{}}}}}", n * 10)
}

fn subscribed() -> String {
    "{\"type\":\"subscribed\",\"topic\":\"Fills\"}".to_string()
}

fn unauthorized(reason: &str) -> String {
    format!("{{\"type\":\"unauthorized\",\"topic\":\"Fills\",\"reason\":\"{reason}\"}}")
}

/// Connect as `token`, subscribe to `Fills`, and expect to be admitted.
fn admitted(server: &Server, token: &str) -> Ws {
    let mut ws = Ws::connect(server.port, Some(token));
    ws.subscribe("Fills");
    assert_eq!(ws.text(), subscribed());
    ws
}

#[test]
fn the_handshake_answers_with_the_key_the_rfc_derives() {
    let server = start();
    let ws = Ws::connect(server.port, Some("t-dave"));
    assert_eq!(ws.accept_key(), RFC_ACCEPT, "{}", ws.head);
    assert!(ws.head.contains("Upgrade: websocket"), "{}", ws.head);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn an_operator_subscribes_and_receives_the_events_in_order_with_increasing_seq() {
    let server = start();
    let mut dave = admitted(&server, "t-dave");
    server.await_stat("subs", 1);
    server.command("fill 5");
    for n in 1..=5 {
        assert_eq!(dave.text(), event(n, n), "event {n}");
    }
    // the sequence continues across publishes
    server.command("fill 2");
    assert_eq!(dave.text(), event(6, 1));
    assert_eq!(dave.text(), event(7, 2));
    dave.silence(150);
    assert_eq!(server.stat("events"), Some(7));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn a_credential_presented_as_a_query_parameter_is_the_same_credential() {
    let server = start();
    let mut ws = Ws::connect_with_query(server.port, "t-dave");
    ws.subscribe("Fills");
    assert_eq!(ws.text(), subscribed());
    server.await_stat("subs", 1);
    server.command("fill 1");
    assert_eq!(ws.text(), event(1, 1));
    server.finish();
}

#[test]
fn a_subscriber_who_may_not_read_a_stream_is_refused_and_buffers_nothing() {
    let server = start();
    let mut bob = Ws::connect(server.port, Some("t-bob"));
    bob.subscribe("Fills");
    assert_eq!(
        bob.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Fills requires operator\",\"requires\":[\"operator\"]}}"
    );
    server.command("fill 3");
    server.await_stat("made", 3);
    // nothing is admitted, so nothing is queued or delivered
    assert_eq!(server.stat("subs"), Some(0));
    bob.silence(300);
    // the refusal did not end the connection: it may ask again, and be refused again
    bob.subscribe("Fills");
    assert!(bob.text().contains("\"kind\":\"unauthorized\""));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn the_other_refusals_are_the_contracts() {
    let server = start();
    // a topic the hub binds no stream for
    let mut dave = Ws::connect(server.port, Some("t-dave"));
    dave.subscribe("Nothing");
    assert_eq!(
        dave.text(),
        "{\"type\":\"refusal\",\"topic\":\"Nothing\",\"refusal\":{\"kind\":\"malformed\",\"reason\":\"unknown_topic: Nothing\"}}"
    );
    // not a frame
    dave.send_text("not json");
    assert_eq!(
        dave.text(),
        "{\"type\":\"refusal\",\"topic\":null,\"refusal\":{\"kind\":\"malformed\",\"reason\":\"a frame is one JSON object\"}}"
    );
    dave.send_text("{\"type\":\"launch\"}");
    assert!(dave.text().contains("unknown frame type: launch"));
    dave.send_text("{\"type\":\"subscribe\"}");
    assert!(dave.text().contains("a subscribe frame names its topic"));
    // and the connection serves on
    dave.subscribe("Fills");
    assert_eq!(dave.text(), subscribed());
    // nobody named at connect
    let mut anon = Ws::connect(server.port, None);
    anon.subscribe("Fills");
    assert_eq!(
        anon.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthenticated\",\"reason\":\"no credential was presented\"}}"
    );
    let mut ghost = Ws::connect(server.port, Some("t-ghost"));
    ghost.subscribe("Fills");
    assert_eq!(
        ghost.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthenticated\",\"reason\":\"the bearer token is refused: no such token\"}}"
    );
    server.finish();
}

#[test]
fn a_ping_is_answered_and_a_client_close_ends_the_connection() {
    let server = start();
    let mut ws = admitted(&server, "t-dave");
    ws.send_ping(b"are you there");
    assert_eq!(ws.recv_within(5000), Frame::Pong(b"are you there".to_vec()));
    ws.send_close();
    assert!(matches!(ws.recv_within(5000), Frame::Close(_)));
    assert_eq!(ws.recv_within(5000), Frame::Eof);
    server.await_stat("subs", 0);
    server.finish();
}

#[test]
fn a_revoked_grant_ends_the_subscription_with_one_frame_and_delivers_nothing_published_after() {
    let server = start();
    let mut dave = admitted(&server, "t-dave");
    let mut erin = admitted(&server, "t-erin");
    server.await_stat("subs", 2);
    server.command("fill 2");
    assert_eq!(dave.text(), event(1, 1));
    assert_eq!(dave.text(), event(2, 2));
    assert_eq!(erin.text(), event(1, 1));
    assert_eq!(erin.text(), event(2, 2));
    // the revision and the next publishes follow each other as closely as the
    // program can make them: none of the three may reach dave
    server.command("revoke dave; fill 3");
    assert_eq!(dave.text(), unauthorized("revoked"));
    // erin still holds `operator`: her stream goes on, her seq continues
    assert_eq!(erin.text(), event(3, 1));
    assert_eq!(erin.text(), event(4, 2));
    assert_eq!(erin.text(), event(5, 3));
    dave.silence(300);
    erin.silence(100);
    server.await_stat("subs", 1);
    // dave's connection is open and he may not subscribe again
    dave.subscribe("Fills");
    assert!(dave.text().contains("\"kind\":\"unauthorized\""));
    server.command("fill 1");
    assert_eq!(erin.text(), event(6, 1));
    dave.silence(200);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn a_revision_that_changes_nothing_for_a_subscriber_interrupts_nothing() {
    let server = start();
    let mut erin = admitted(&server, "t-erin");
    server.await_stat("subs", 1);
    // the hub is told grants changed; they did not, for erin
    server.command("announce; fill 2; announce; fill 1");
    assert_eq!(erin.text(), event(1, 1));
    assert_eq!(erin.text(), event(2, 2));
    assert_eq!(erin.text(), event(3, 1));
    erin.silence(150);
    assert_eq!(server.stat("subs"), Some(1));
    server.finish();
}

#[test]
fn an_expired_credential_ends_the_subscription_at_its_instant() {
    let server = start();
    let t0 = Instant::now();
    let mut soon = admitted(&server, "t-soon");
    server.await_stat("subs", 1);
    server.command("fill 1");
    assert_eq!(soon.text(), event(1, 1));
    // one `unauthorized` frame at the credential's expiry, with no event or
    // request to wake the connection
    assert_eq!(soon.text(), unauthorized("expired"));
    let at = t0.elapsed();
    assert!(at >= Duration::from_millis(500) && at < Duration::from_millis(3000), "expired after {at:?}");
    server.await_stat("subs", 0);
    // nothing published after is delivered, and the spent credential admits nothing new
    server.command("fill 3");
    server.await_stat("made", 4);
    soon.silence(300);
    soon.subscribe("Fills");
    let refusal = soon.text();
    assert!(refusal.contains("\"kind\":\"unauthenticated\"") && refusal.contains("expired"), "{refusal}");
    server.finish();
}

#[test]
fn stop_sends_closed_to_every_subscriber_and_releases_the_address() {
    let server = start();
    let port = server.port;
    let mut dave = admitted(&server, "t-dave");
    let mut erin = admitted(&server, "t-erin");
    server.await_stat("subs", 2);
    server.command("fill 2");
    assert_eq!(dave.text(), event(1, 1));
    assert_eq!(dave.text(), event(2, 2));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    // what was published before stop() arrives before `closed`, then the close
    assert_eq!(erin.text(), event(1, 1));
    assert_eq!(erin.text(), event(2, 2));
    for ws in [&mut dave, &mut erin] {
        assert_eq!(ws.text(), "{\"type\":\"closed\",\"reason\":\"shutting_down\"}");
        assert!(matches!(ws.recv_within(5000), Frame::Close(_)));
        assert_eq!(ws.recv_within(5000), Frame::Eof);
    }
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "the address is released");
}

#[test]
fn a_program_that_ends_without_stop_ends_its_connections_with_no_closed_frame() {
    let server = Server::start(&build(), &[("MODE", "scope_exit")]);
    let mut dave = admitted(&server, "t-dave");
    server.await_stat("subs", 1);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    // the transport failure: the connection ends and no `closed` frame came
    loop {
        match dave.recv_within(5000) {
            Frame::Eof => break,
            Frame::Text(t) => assert!(!t.contains("closed"), "{t}"),
            Frame::Close(_) => {}
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_connection_that_breaks_is_forgotten_and_the_others_are_served() {
    let server = start();
    let dave = admitted(&server, "t-dave");
    let mut erin = admitted(&server, "t-erin");
    server.await_stat("subs", 2);
    server.await_stat("conns", 2);
    // no closing handshake
    dave.abandon();
    server.await_stat("subs", 1);
    server.await_stat("conns", 1);
    server.command("fill 2");
    assert_eq!(erin.text(), event(1, 1));
    assert_eq!(erin.text(), event(2, 2));
    // a half-open handshake and a connection that sends garbage do not trouble it either
    {
        use std::io::Write;
        let mut raw = std::net::TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
        raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").expect("half a request");
    }
    {
        use std::io::Write;
        let mut raw = std::net::TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
        raw.write_all(b"\x00\x01\x02 not http\r\n\r\n").expect("garbage");
    }
    server.command("fill 1");
    assert_eq!(erin.text(), event(3, 1));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

/// The fixture documents beside the witness program, as values.
fn fixture(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/api-contract").join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("json")
}

#[test]
fn the_description_from_the_hubs_listener_is_the_fixture_for_the_caller() {
    // the witness's own address is in its fixtures; the program is built for the one a test can have
    let port = ports::free_port();
    let src = WITNESS.replace("std::env::var(\"BIND\")", &format!("\"127.0.0.1:{port}\""));
    let bin = harness::unique_bin("api_hub_streams_desc");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let dir = harness::unique_dir("wshub_desc");
    let server = Server::start_on(&bin, &[], port, dir);
    let start = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(start.elapsed() < Duration::from_secs(20), "the hub never listened");
        std::thread::sleep(Duration::from_millis(10));
    }
    for (token, name) in [("t-dave", "fills.dave.description.json"), ("t-bob", "fills.bob.description.json")] {
        let (status, body) = get_description(port, Some(token));
        assert_eq!(status, 200, "{body}");
        let got: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"));
        let mut want = fixture(name);
        // (the listener's address is the one this program was built for)
        want["listener"]["address"] = serde_json::Value::String(format!("127.0.0.1:{port}"));
        assert_eq!(got, want, "the description for {token}");
        // served as the compact document, keys in the schema's order
        assert!(body.starts_with("{\"description\":1,\"exposure\":\"hub@fnv1a64:26970854397ab154/fills\""), "{body}");
        assert!(!body.contains('\n'));
    }
    // a caller the sources name nobody for has no description
    let (status, body) = get_description(port, None);
    assert_eq!(status, 401, "{body}");
    assert!(body.contains("\"kind\":\"unauthenticated\""), "{body}");
    let (status, _) = get_description(port, Some("t-ghost"));
    assert_eq!(status, 401);
    // (the stream digest the description names is the one the compiler derives:
    // `digest.md`'s, which the fixtures carry)
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

/// The same scenarios under AddressSanitizer (with the arena's chunk recycling
/// off, so a use after free reads what the sanitizer can see): a revocation
/// with events behind it, an expiry, a connection that breaks, and a stop.
#[test]
fn the_revocation_and_expiry_program_runs_clean_under_asan() {
    let bin = harness::unique_bin("api_hub_streams_asan");
    harness::build_source_asan(WITNESS, &bin);
    let server = Server::start(&bin, &[]);
    let mut dave = admitted(&server, "t-dave");
    let mut erin = admitted(&server, "t-erin");
    let broken = admitted(&server, "t-erin");
    let mut soon = admitted(&server, "t-soon");
    server.await_stat("subs", 4);
    server.command("fill 3");
    for ws in [&mut dave, &mut erin, &mut soon] {
        assert_eq!(ws.text(), event(1, 1));
        assert_eq!(ws.text(), event(2, 2));
        assert_eq!(ws.text(), event(3, 3));
    }
    broken.abandon();
    server.command("revoke dave; fill 4");
    assert_eq!(dave.text(), unauthorized("revoked"));
    for n in 1..=4 {
        assert_eq!(erin.text(), event(3 + n, n));
    }
    // the credential that expires ends its subscription, with the events published before it delivered
    let mut seen = Vec::new();
    loop {
        let t = soon.text();
        if t == unauthorized("expired") {
            break;
        }
        seen.push(t);
    }
    assert_eq!(seen.len(), 4, "{seen:?}");
    dave.silence(200);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert_eq!(erin.text(), "{\"type\":\"closed\",\"reason\":\"shutting_down\"}");
}
