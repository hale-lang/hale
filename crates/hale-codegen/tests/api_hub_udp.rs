//! GH #1417 (R5): the stream half over datagrams (`std::api::udp::Hub`).
//!
//! The witness's hub half (`support/hub_witness.rs`) with `udp::Hub` for
//! `ws::Hub`. A datagram hub speaks the `ws` frames minus the connection:
//! every datagram of a subscriber carries the id the subscriber chose in its
//! subscribe, the credential rides the subscribe (there is no connection to
//! authenticate once at connect), and expiry and revocation send the
//! `unauthorized` datagram. A request and its reply over datagrams are not
//! framed in this release (R6): the serve-site law says so.

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
#[path = "support/udp_client.rs"]
mod udp_client;
#[path = "support/hub_witness.rs"]
mod hub_witness;

use hub_witness::WITNESS;
use udp_client::*;

fn program() -> String {
    WITNESS.replace("ws::Hub", "udp::Hub")
}

fn build() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = harness::unique_bin("api_hub_udp");
        build_opts::build_source(&program(), &bin, &build_opts::options()).expect("build the datagram hub program");
        bin
    })
    .clone()
}

fn event(id: &str, seq: i64, n: i64) -> String {
    format!(
        "{{\"type\":\"event\",\"id\":{id},\"topic\":\"Fills\",\"seq\":{seq},\"payload\":{{\"order\":{n},\"qty\":{n},\"price\":{}}}}}",
        n * 10
    )
}

fn subscribed(id: &str) -> String {
    format!("{{\"type\":\"subscribed\",\"id\":{id},\"topic\":\"Fills\"}}")
}

fn unauthorized(id: &str, reason: &str) -> String {
    format!("{{\"type\":\"unauthorized\",\"id\":{id},\"topic\":\"Fills\",\"reason\":\"{reason}\"}}")
}

/// A subscriber admitted as `token` under `id`.
fn admitted(server: &ws_hub::Server, token: &str, id: &str) -> Udp {
    let c = Udp::to(server.port);
    c.subscribe("Fills", id, Some(token));
    assert_eq!(c.text(), subscribed(id));
    c
}

#[test]
fn a_subscriber_is_admitted_by_a_datagram_and_receives_events_under_its_id() {
    let server = start(&build(), &[]);
    let dave = admitted(&server, "t-dave", "\"s1\"");
    server.await_stat("subs", 1);
    server.command("fill 3");
    for n in 1..=3 {
        assert_eq!(dave.text(), event("\"s1\"", n, n));
    }
    server.command("fill 1");
    assert_eq!(dave.text(), event("\"s1\"", 4, 1));
    dave.silence(150);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn a_number_is_an_id_too_and_two_ids_of_one_socket_are_two_subscribers() {
    let server = start(&build(), &[]);
    let c = Udp::to(server.port);
    c.subscribe("Fills", "7", Some("t-dave"));
    assert_eq!(c.text(), subscribed("7"));
    c.subscribe("Fills", "\"other\"", Some("t-erin"));
    assert_eq!(c.text(), subscribed("\"other\""));
    server.await_stat("subs", 2);
    server.command("fill 2");
    let mut got = vec![c.text(), c.text(), c.text(), c.text()];
    got.sort();
    let mut want = vec![event("7", 1, 1), event("7", 2, 2), event("\"other\"", 1, 1), event("\"other\"", 2, 2)];
    want.sort();
    assert_eq!(got, want);
    server.finish();
}

#[test]
fn a_subscriber_who_may_not_read_is_refused_and_gets_nothing() {
    let server = start(&build(), &[]);
    let bob = Udp::to(server.port);
    bob.subscribe("Fills", "\"b\"", Some("t-bob"));
    assert_eq!(
        bob.text(),
        "{\"type\":\"refusal\",\"id\":\"b\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Fills requires operator\",\"requires\":[\"operator\"]}}"
            .replace("\"id\":\"b\"", "\"id\":\"b\"")
    );
    server.command("fill 3");
    server.await_stat("made", 3);
    assert_eq!(server.stat("subs"), Some(0));
    bob.silence(300);
    // the other refusals
    let anon = Udp::to(server.port);
    anon.subscribe("Fills", "1", None);
    assert!(anon.text().contains("\"kind\":\"unauthenticated\",\"reason\":\"no credential was presented\""));
    let ghost = Udp::to(server.port);
    ghost.subscribe("Fills", "1", Some("t-ghost"));
    assert!(ghost.text().contains("\"reason\":\"the bearer token is refused: no such token\""));
    let dave = Udp::to(server.port);
    dave.subscribe("Nothing", "1", Some("t-dave"));
    assert!(dave.text().contains("\"kind\":\"malformed\",\"reason\":\"unknown_topic: Nothing\""));
    server.finish();
}

#[test]
fn a_revoked_grant_sends_one_unauthorized_datagram_and_nothing_published_after() {
    let server = start(&build(), &[]);
    let dave = admitted(&server, "t-dave", "\"d\"");
    let erin = admitted(&server, "t-erin", "\"e\"");
    server.await_stat("subs", 2);
    server.command("fill 2");
    assert_eq!(dave.text(), event("\"d\"", 1, 1));
    assert_eq!(dave.text(), event("\"d\"", 2, 2));
    assert_eq!(erin.text(), event("\"e\"", 1, 1));
    assert_eq!(erin.text(), event("\"e\"", 2, 2));
    server.command("revoke dave; fill 3");
    assert_eq!(dave.text(), unauthorized("\"d\"", "revoked"));
    for n in 1..=3 {
        assert_eq!(erin.text(), event("\"e\"", 2 + n, n));
    }
    dave.silence(300);
    server.await_stat("subs", 1);
    server.finish();
}

#[test]
fn an_expired_credential_sends_unauthorized_at_its_instant() {
    let server = start(&build(), &[]);
    let t0 = std::time::Instant::now();
    let soon = admitted(&server, "t-soon", "\"x\"");
    server.await_stat("subs", 1);
    server.command("fill 1");
    assert_eq!(soon.text(), event("\"x\"", 1, 1));
    assert_eq!(soon.text(), unauthorized("\"x\"", "expired"));
    let at = t0.elapsed();
    assert!(at >= std::time::Duration::from_millis(500) && at < std::time::Duration::from_millis(3000), "{at:?}");
    server.await_stat("subs", 0);
    server.command("fill 2");
    server.await_stat("made", 3);
    soon.silence(250);
    server.finish();
}

#[test]
fn unsubscribe_ends_the_subscription_and_the_id_may_be_used_again() {
    let server = start(&build(), &[]);
    let c = admitted(&server, "t-dave", "\"u\"");
    server.await_stat("subs", 1);
    c.send("{\"type\":\"unsubscribe\",\"id\":\"u\"}");
    server.await_stat("subs", 0);
    server.command("fill 2");
    server.await_stat("made", 2);
    c.silence(250);
    // the same id, afresh: seq starts again at 1
    c.subscribe("Fills", "\"u\"", Some("t-dave"));
    assert_eq!(c.text(), subscribed("\"u\""));
    server.command("fill 1");
    assert_eq!(c.text(), event("\"u\"", 1, 1));
    server.finish();
}

#[test]
fn stop_sends_closed_to_every_subscriber() {
    let server = start(&build(), &[]);
    let dave = admitted(&server, "t-dave", "\"d\"");
    let erin = admitted(&server, "t-erin", "\"e\"");
    server.await_stat("subs", 2);
    server.command("fill 1");
    assert_eq!(dave.text(), event("\"d\"", 1, 1));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert_eq!(dave.text(), "{\"type\":\"closed\",\"id\":\"d\",\"reason\":\"shutting_down\"}");
    assert_eq!(erin.text(), event("\"e\"", 1, 1));
    assert_eq!(erin.text(), "{\"type\":\"closed\",\"id\":\"e\",\"reason\":\"shutting_down\"}");
}

#[test]
fn a_datagram_asks_for_the_description() {
    // built for a literal address, as the description names it (and the witness's fixtures have it)
    let port = free_udp_port();
    let src = program().replace("std::env::var(\"BIND\")", &format!("\"127.0.0.1:{port}\""));
    let bin = harness::unique_bin("api_hub_udp_desc");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let server = ws_hub::Server::start_on(&bin, &[], port, harness::unique_dir("udphub_desc"));
    let begin = std::time::Instant::now();
    while server.stat("made").is_none() {
        assert!(begin.elapsed() < std::time::Duration::from_secs(30), "the hub never bound");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let c = Udp::to(port);
    let ask = |token: Option<&str>, id: &str| {
        let token = token.map(|t| format!(",\"token\":\"{t}\"")).unwrap_or_default();
        c.send(&format!("{{\"type\":\"describe\",\"id\":\"{id}\"{token}}}"));
        serde_json::from_str::<serde_json::Value>(&c.text()).expect("a JSON datagram")
    };
    for (token, name) in [("t-dave", "fills.dave.description.json"), ("t-bob", "fills.bob.description.json")] {
        let got = ask(Some(token), "q");
        assert_eq!(got["type"], "description");
        assert_eq!(got["id"], "q");
        let mut doc = got["document"].clone();
        // the fixtures are the `ws` hub's: the udp form differs in its listener, its outcome frames and its loss statement
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/api-contract").join(name);
        let mut want: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(doc["listener"], serde_json::json!({"transport": "udp", "address": format!("127.0.0.1:{port}")}));
        assert_eq!(doc["outcomes"]["transport"], "udp");
        assert_eq!(doc["outcomes"]["subscribe"]["fields"], serde_json::json!(["topic", "id", "token"]));
        assert_eq!(doc["outcomes"]["event"]["fields"], serde_json::json!(["topic", "id", "seq", "payload"]));
        assert_eq!(doc["outcomes"]["closed"]["fields"], serde_json::json!(["id", "reason"]));
        for v in [&mut doc, &mut want] {
            v["listener"] = serde_json::Value::Null;
            v["outcomes"] = serde_json::Value::Null;
            if let Some(streams) = v["streams"].as_array_mut() {
                for st in streams {
                    assert!(st["loss"].is_string());
                    st["loss"] = serde_json::Value::Null;
                }
            }
        }
        assert_eq!(doc, want, "the description for {token}");
    }
    let got = ask(None, "n");
    assert_eq!(got["document"], serde_json::Value::Null);
    assert_eq!(got["refusal"]["refusal"]["kind"], "unauthenticated");
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn a_hub_that_cannot_bind_its_address_fails_the_boot() {
    // (an address no interface holds: UDP sockets share a port, so a port in use is not a refusal)
    let mut p = ws_hub::Server::start_on(
        &build(),
        &[("BIND", "203.0.113.7:9")],
        9,
        harness::unique_dir("udphub_unbindable"),
    );
    let done = p.wait(std::time::Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "{}{}", done.stdout, done.stderr);
    assert!(done.stderr.contains("api: udp::Hub could not listen on 203.0.113.7:9"), "{}", done.stderr);
}
