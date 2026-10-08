//! GH #1417 (R5): a subscriber that cannot keep up loses events, not the
//! subscription; and a hub that also serves a surface carries rpcs on the
//! connection a subscriber holds.
//!
//! `DESK` binds two streams to one hub (`Blobs`, `bound: 4, on_full:
//! drop_old`; `Ticks`, `bound: 4, on_full: drop_new`), each carrying a
//! payload (60,000 bytes: an event has to fit the bus frame of 65,536) large
//! enough that a client who does not read fills the socket and then the queue, and serves `Public` over the same hub (`api::serve(Public,
//! self.hub, …)`). Spec/api.md § Streams: a subscription's queue holds at
//! most `bound` frames and sheds under `on_full`; every event offered to it
//! takes the next `seq`, so a gap in `seq` is exactly the frames shed.

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
#[path = "support/ws_client.rs"]
mod ws_client;

use ws_client::*;
use ws_hub::*;

pub const DESK: &str = r#"
role operator;

type Blob { n: Int; data: String; }
topic Blobs { payload: Blob; subject: "desk.blobs"; }
topic Ticks { payload: Blob; subject: "desk.ticks"; }

type PlaceOrder { qty: Int; }
type Receipt { order: Int; qty: Int; }
type Ask { n: Int; }
type Stats { placed: Int; }

api Public {
    rpc Orders::place;
    rpc Orders::stats requires: [operator];
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

locus Orders {
    params { placed: Int = 0; }
    fn place(o: PlaceOrder) -> Receipt {
        self.placed = self.placed + 1;
        return Receipt { order: 100 + self.placed, qty: o.qty };
    }
    fn stats(a: Ask) -> Stats {
        return Stats { placed: self.placed };
    }
}

locus Maker {
    params { made: Int = 0; }
    bus { publish Blobs; publish Ticks; }
    fn blobs(n: Int, size: Int) {
        let mut d = "x";
        while len(d) < size { d = d + d; }
        d = d[0..size];
        let mut i = 1;
        while i <= n {
            self.made = self.made + 1;
            Blobs <- Blob { n: i, data: d };
            i = i + 1;
        }
    }
    fn ticks(n: Int, size: Int) {
        let mut d = "y";
        while len(d) < size { d = d + d; }
        d = d[0..size];
        let mut i = 1;
        while i <= n {
            self.made = self.made + 1;
            Ticks <- Blob { n: i, data: d };
            i = i + 1;
        }
    }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave" };
        hub: ws::Hub = ws::Hub { bind: std::env::var("BIND"), principals: self.bearer, roles: self.hub_roles, as: "feed" };
        orders: Orders = Orders { };
        maker: Maker = Maker { };
    }
    placement { orders: cooperative(pool = work) where async_io; }
    bindings {
        Blobs: self.hub requires: [operator], bound: 4, on_full: drop_old;
        Ticks: self.hub requires: [operator], bound: 4, on_full: drop_new;
    }
    fn stats(ctl: String) {
        std::io::fs::write_file(ctl + "/stats.tmp", "made=" + to_string(self.maker.made) + "\nsubs=" + to_string(self.hub.subscribers()) + "\nconns=" + to_string(self.hub.connections()) + "\nplaced=" + to_string(self.orders.placed) + "\n") or discard;
        std::io::fs::rename(ctl + "/stats.tmp", ctl + "/stats") or discard;
    }
    // `blobs N SIZE`, `ticks N SIZE`, `stop`
    fn step(s: String) -> Bool {
        let c = std::str::trim(s);
        if c == "stop" { return true; }
        if std::str::starts_with(c, "blobs ") || std::str::starts_with(c, "ticks ") {
            let rest = c[6..len(c)];
            let sp = std::str::index_of(rest, " ");
            let n = std::str::parse_int(rest[0..sp]) or 0;
            let size = std::str::parse_int(rest[(sp + 1)..len(rest)]) or 0;
            if std::str::starts_with(c, "blobs ") { self.maker.blobs(n, size); } else { self.maker.ticks(n, size); }
        }
        return false;
    }
    run() {
        let desk = api::serve(Public, self.hub, as: "desk", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
        let ctl = std::env::var("CTL");
        let mut done = false;
        while !done && !self.draining {
            let cmd = std::str::trim(std::io::fs::read_file(ctl + "/cmd") or "");
            if len(cmd) > 0 {
                std::io::fs::unlink(ctl + "/cmd") or discard;
                if self.step(cmd) { done = true; }
            }
            self.stats(ctl);
            std::time::sleep(5ms);
        }
        println("placed=", self.orders.placed, " made=", self.maker.made);
        desk.stop();
        self.hub.stop();
    }
}

fn main() { Desk { }; }
"#;

fn build(asan: bool) -> PathBuf {
    static PLAIN: OnceLock<PathBuf> = OnceLock::new();
    static SANITIZED: OnceLock<PathBuf> = OnceLock::new();
    let cell = if asan { &SANITIZED } else { &PLAIN };
    cell.get_or_init(|| {
        let bin = harness::unique_bin(if asan { "api_hub_desk_asan" } else { "api_hub_desk" });
        if asan {
            harness::build_source_asan(DESK, &bin);
        } else {
            build_opts::build_source(DESK, &bin, &build_opts::options()).expect("build the desk program");
        }
        bin
    })
    .clone()
}

/// The `seq` of an event frame, and its payload's `n`.
fn seq_and_n(frame: &str) -> (i64, i64) {
    let int = |key: &str| -> i64 {
        let at = frame.find(key).unwrap_or_else(|| panic!("no {key} in {}", &frame[..frame.len().min(200)])) + key.len();
        frame[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()
    };
    (int("\"seq\":"), int("\"n\":"))
}

/// Subscribe to `topic` as dave, publish `n` frames of `size` bytes with
/// the client not reading, then read until the stream is quiet: the
/// `(seq, n)` of every event that arrived.
fn burst(asan: bool, topic: &str, command: &str) -> Vec<(i64, i64)> {
    let server = Server::start(&build(asan), &[]);
    let mut dave = Ws::connect(server.port, Some("t-dave"));
    dave.subscribe(topic);
    assert_eq!(dave.text(), format!("{{\"type\":\"subscribed\",\"topic\":\"{topic}\"}}"));
    server.await_stat("subs", 1);
    // the client does not read while the program publishes
    server.command(command);
    server.await_stat("made", 400);
    let mut got = Vec::new();
    let first = dave.text();
    let mut frames = vec![first];
    frames.extend(dave.texts_until_quiet(800));
    for frame in frames {
        assert!(frame.contains(&format!("\"topic\":\"{topic}\"")), "{}", &frame[..frame.len().min(200)]);
        got.push(seq_and_n(&frame));
    }
    // the next event offered, once the reader has caught up, carries the
    // next seq: what lies between it and the last frame read is what was shed
    let again = if topic == "Blobs" { "blobs 1 100" } else { "ticks 1 100" };
    server.command(again);
    let (seq, n) = seq_and_n(&dave.text());
    assert_eq!((seq, n), (401, 1), "seq counts every event offered, delivered or shed");
    got.push((seq, 401));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    got
}

fn assert_shed(got: &[(i64, i64)]) {
    assert!(!got.is_empty());
    // in publish order, seq strictly increasing; the payload's own counter is the seq
    // (one subscription, nothing else offered to it)
    for w in got.windows(2) {
        assert!(w[0].0 < w[1].0, "seq is not increasing: {w:?}");
    }
    for (seq, n) in got {
        assert_eq!(seq, n, "the event with seq {seq} carries payload {n}");
    }
}

#[test]
fn drop_old_sheds_the_oldest_and_the_seq_gap_is_the_frames_shed() {
    let got = burst(false, "Blobs", "blobs 400 60000");
    assert_shed(&got);
    // frames were shed: fewer arrived than were offered, and there is a gap in seq
    assert!(got.len() < 400, "nothing was shed: {} frames", got.len());
    let gaps: i64 = got.windows(2).map(|w| w[1].0 - w[0].0 - 1).sum();
    assert!(gaps > 0, "no gap in seq");
    // the frames held back are the newest: the last one of the burst is delivered
    assert_eq!(got[got.len() - 2].0, 400, "drop_old keeps the newest");
    // seq counts every event offered: what arrived and what was shed are all of them
    assert_eq!(got.len() as i64 + gaps + (got[0].0 - 1), 401);
}

#[test]
fn drop_new_sheds_the_frame_being_published() {
    let got = burst(false, "Ticks", "ticks 400 60000");
    assert_shed(&got);
    assert!(got.len() < 400, "nothing was shed: {} frames", got.len());
    let gaps: i64 = got.windows(2).map(|w| w[1].0 - w[0].0 - 1).sum();
    assert!(gaps > 0, "no gap in seq");
    // the oldest are kept: the first frames offered arrive, the newest are the ones shed
    // (the gap is between the last of them and the event offered after the reader caught up)
    assert_eq!(got[0].0, 1, "drop_new keeps the oldest");
    assert!(got[got.len() - 2].0 < 400, "the newest frame survived a client who did not read");
    assert_eq!(got.len() as i64 + gaps + (got[0].0 - 1), 401);
}

#[test]
fn the_shedding_program_runs_clean_under_asan() {
    let got = burst(true, "Blobs", "blobs 400 60000");
    assert_shed(&got);
    assert!(got.len() < 400);
    let got = burst(true, "Ticks", "ticks 400 60000");
    assert_shed(&got);
}

#[test]
fn a_reader_who_keeps_up_loses_nothing_and_a_slow_one_does_not_stop_it() {
    let server = Server::start(&build(false), &[]);
    let mut slow = Ws::connect(server.port, Some("t-dave"));
    let mut fast = Ws::connect(server.port, Some("t-dave"));
    for ws in [&mut slow, &mut fast] {
        ws.subscribe("Blobs");
        assert!(ws.text().contains("subscribed"));
    }
    server.await_stat("subs", 2);
    // modest frames the client reads as they come: nothing is shed for it
    server.command("blobs 40 200");
    for n in 1..=40 {
        let (seq, got) = seq_and_n(&fast.text());
        assert_eq!((seq, got), (n, n));
    }
    // the slow reader read none yet and lost none either: bound 4 of 40 small
    // frames would shed if the socket did not hold them
    let held: Vec<(i64, i64)> = slow.texts_until_quiet(300).iter().map(|f| seq_and_n(f)).collect();
    assert_eq!(held.len(), 40);
    server.finish();
}

#[test]
fn a_hub_that_serves_a_surface_carries_rpcs_on_the_subscribers_connection() {
    let server = Server::start(&build(false), &[]);
    let mut dave = Ws::connect(server.port, Some("t-dave"));
    dave.subscribe("Blobs");
    assert!(dave.text().contains("subscribed"));
    server.await_stat("subs", 1);
    // an rpc as the Context the connection was authenticated as
    dave.send_text("{\"type\":\"call\",\"id\":\"c1\",\"call\":\"Orders::place\",\"payload\":{\"qty\":3}}");
    let reply = dave.text();
    assert!(reply.starts_with("{\"type\":\"reply\",\"request_id\":"), "{reply}");
    assert!(reply.contains("\"id\":\"c1\",\"ok\":true,\"value\":{\"order\":101,\"qty\":3}"), "{reply}");
    assert!(reply.contains("\"caller\":{\"mode\":\"bearer\",\"name\":\"dave\""), "{reply}");
    // and events and replies share the connection
    server.command("blobs 3 100");
    dave.send_text("{\"type\":\"call\",\"id\":\"c2\",\"call\":\"Orders::stats\",\"payload\":{\"n\":0}}");
    let mut events = 0;
    let mut stats = None;
    while events < 3 || stats.is_none() {
        let f = dave.text();
        if f.contains("\"type\":\"event\"") {
            events += 1;
        } else {
            stats = Some(f);
        }
    }
    assert!(stats.unwrap().contains("\"id\":\"c2\",\"ok\":true,\"value\":{\"placed\":1}"));
    // a caller who does not hold the member's role is refused before the handler runs
    let mut bob = Ws::connect(server.port, Some("t-bob"));
    bob.send_text("{\"type\":\"call\",\"id\":\"b1\",\"call\":\"Orders::stats\",\"payload\":{\"n\":0}}");
    let refusal = bob.text();
    assert!(refusal.contains("\"id\":\"b1\",\"ok\":false,\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Orders::stats requires operator\""), "{refusal}");
    // and one the sources name nobody for
    let mut anon = Ws::connect(server.port, None);
    anon.send_text("{\"type\":\"call\",\"id\":\"a1\",\"call\":\"Orders::place\",\"payload\":{\"qty\":1}}");
    let refusal = anon.text();
    assert!(refusal.contains("\"ok\":false,\"refusal\":{\"kind\":\"unauthenticated\""), "{refusal}");
    // nothing a refused caller did reached the handler
    server.command("blobs 1 10");
    server.await_stat("placed", 1);
    // a call that is not one
    bob.send_text("{\"type\":\"call\",\"id\":\"b2\",\"payload\":{}}");
    assert!(bob.text().contains("\"kind\":\"malformed\""));
    bob.send_text("{\"type\":\"call\",\"id\":\"b3\",\"call\":\"Orders::nothing\",\"payload\":{}}");
    assert!(bob.text().contains("unknown_member: Orders::nothing"));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert!(done.stdout.contains("placed=1"), "{}", done.stdout);
}

#[test]
fn stopping_the_serve_handle_stops_the_hub_and_closes_the_subscribers() {
    let server = Server::start(&build(false), &[]);
    let mut dave = Ws::connect(server.port, Some("t-dave"));
    dave.subscribe("Ticks");
    assert!(dave.text().contains("subscribed"));
    server.await_stat("subs", 1);
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert_eq!(dave.text(), "{\"type\":\"closed\",\"reason\":\"shutting_down\"}");
    assert!(matches!(dave.recv_within(5000), Frame::Close(_)));
    assert_eq!(dave.recv_within(5000), Frame::Eof);
}
